//! Tool-policy pattern matching and owner-restrictive resolution.

use serde::{Deserialize, Serialize};

use crate::{Owner, PolicyId, generate_key_between};

/// Gate applied when a pattern matches a tool id or address.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    /// Always allowed.
    Approve,
    /// Pause until a human accepts.
    RequireApproval,
    /// Reject. Outer `block` cannot be weakened inward.
    Block,
}

impl PolicyAction {
    /// Higher is more restrictive.
    #[must_use]
    pub const fn restriction_rank(self) -> u8 {
        match self {
            Self::Approve => 1,
            Self::RequireApproval => 2,
            Self::Block => 3,
        }
    }

    /// Pick the more restrictive of two actions.
    #[must_use]
    pub const fn tighten(self, other: Self) -> Self {
        if other.restriction_rank() > self.restriction_rank() {
            other
        } else {
            self
        }
    }
}

/// Validated pattern. Invalid wildcards cannot be constructed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PolicyPattern(String);

impl PolicyPattern {
    /// Parse the policy grammar.
    ///
    /// # Errors
    ///
    /// Empty, leading `*` other than bare `*`, partial wildcards, empty segments.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, String> {
        let pattern = raw.as_ref();
        if is_valid_pattern(pattern) {
            Ok(Self(pattern.to_owned()))
        } else {
            Err(format!("invalid policy pattern: {pattern}"))
        }
    }

    /// Borrow the pattern text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PolicyPattern {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PolicyPattern> for String {
    fn from(value: PolicyPattern) -> Self {
        value.0
    }
}

/// Stored policy row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolPolicy {
    /// Rule id.
    pub id: PolicyId,
    /// Owning side.
    pub owner: Owner,
    /// Match pattern.
    pub pattern: PolicyPattern,
    /// Gate.
    pub action: PolicyAction,
    /// Fractional-index key. Lower lex = higher precedence.
    pub position: String,
}

/// A matched rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyMatch {
    /// Action of the winning rule.
    pub action: PolicyAction,
    /// Pattern text.
    pub pattern: String,
    /// Policy id.
    pub policy_id: String,
}

/// Whether the effective gate came from a user rule or a plugin default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicySource {
    /// A stored `tool_policy` row.
    User,
    /// Plugin `requiresApproval` (or approve-by-default).
    PluginDefault,
}

/// Resolved gate for one tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectivePolicy {
    /// Action to enforce.
    pub action: PolicyAction,
    /// Provenance.
    pub source: PolicySource,
    /// Pattern when source is user.
    pub pattern: Option<String>,
    /// Policy id when source is user.
    pub policy_id: Option<String>,
}

impl EffectivePolicy {
    const fn plugin_default(requires_approval: bool) -> Self {
        Self {
            action: if requires_approval {
                PolicyAction::RequireApproval
            } else {
                PolicyAction::Approve
            },
            source: PolicySource::PluginDefault,
            pattern: None,
            policy_id: None,
        }
    }
}

/// Grammar check. See crate docs for the pattern language.
#[must_use]
pub fn is_valid_pattern(pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if pattern.starts_with('.') || pattern.ends_with('.') || pattern.contains("..") {
        return false;
    }
    if pattern.starts_with('*') {
        return false;
    }
    pattern
        .split('.')
        .all(|seg| !seg.is_empty() && (seg == "*" || !seg.contains('*')))
}

/// Match `pattern` against a tool id or address.
#[must_use]
pub fn match_pattern(pattern: &str, tool_id: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let pattern_segments: Vec<&str> = pattern.split('.').collect();
    let tool_segments: Vec<&str> = tool_id.split('.').collect();
    for (i, seg) in pattern_segments.iter().enumerate() {
        if *seg == "*" {
            if i == pattern_segments.len() - 1 {
                return tool_segments.len() >= i;
            }
            if i >= tool_segments.len() {
                return false;
            }
            continue;
        }
        if i >= tool_segments.len() || tool_segments[i] != *seg {
            return false;
        }
    }
    pattern_segments.len() == tool_segments.len()
}

/// Specificity score used to place new rules.
///
/// `*` → 0; `vercel.*` → 2; `vercel.dns.*` → 4; `vercel.dns` → 5; `vercel.dns.create` → 7.
#[must_use]
pub fn pattern_specificity(pattern: &str) -> u32 {
    if pattern == "*" {
        return 0;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return u32::try_from(prefix.split('.').count())
            .unwrap_or(u32::MAX)
            .saturating_mul(2);
    }
    u32::try_from(pattern.split('.').count())
        .unwrap_or(u32::MAX)
        .saturating_mul(2)
        .saturating_add(1)
}

/// Compare two policy rows: position lex, then id.
#[must_use]
pub fn compare_policy_row(
    a_position: &str,
    a_id: &str,
    b_position: &str,
    b_id: &str,
) -> std::cmp::Ordering {
    a_position.cmp(b_position).then_with(|| a_id.cmp(b_id))
}

/// Default position for a new pattern among an owner's committed rules.
#[must_use]
pub fn position_for_new_pattern(pattern: &str, rows: &[(String, String, String)]) -> String {
    // rows: (pattern, position, id)
    let mut committed = rows.to_vec();
    committed.sort_by(|a, b| compare_policy_row(&a.1, &a.2, &b.1, &b.2));
    let new_score = pattern_specificity(pattern);
    let idx = committed
        .iter()
        .position(|r| pattern_specificity(&r.0) <= new_score)
        .unwrap_or(committed.len());
    let prev = idx
        .checked_sub(1)
        .and_then(|i| committed.get(i).map(|r| r.1.as_str()));
    let next = committed.get(idx).map(|r| r.1.as_str());
    generate_key_between(prev, next)
}

fn more_restrictive(current: Option<PolicyMatch>, candidate: PolicyMatch) -> PolicyMatch {
    match current {
        None => candidate,
        Some(cur) if candidate.action.restriction_rank() > cur.action.restriction_rank() => {
            candidate
        }
        Some(cur) => cur,
    }
}

/// Each owner contributes its first matching rule by local position; the most
/// restrictive match across owners wins.
#[must_use]
pub fn resolve_tool_policy(
    tool_id: &str,
    policies: &[ToolPolicy],
    owner_rank: impl Fn(Owner) -> u8,
) -> Option<PolicyMatch> {
    if policies.is_empty() {
        return None;
    }
    let mut sorted: Vec<&ToolPolicy> = policies.iter().collect();
    sorted.sort_by(|a, b| {
        owner_rank(a.owner).cmp(&owner_rank(b.owner)).then_with(|| {
            compare_policy_row(&a.position, a.id.as_str(), &b.position, b.id.as_str())
        })
    });
    let mut first_by_owner: Vec<(Owner, PolicyMatch)> = Vec::new();
    for row in sorted {
        if first_by_owner.iter().any(|(owner, _)| *owner == row.owner) {
            continue;
        }
        if match_pattern(row.pattern.as_str(), tool_id) {
            first_by_owner.push((
                row.owner,
                PolicyMatch {
                    action: row.action,
                    pattern: row.pattern.as_str().to_owned(),
                    policy_id: row.id.as_str().to_owned(),
                },
            ));
        }
    }
    let mut selected = None;
    for (_, m) in first_by_owner {
        selected = Some(more_restrictive(selected, m));
    }
    selected
}

/// User rules plus plugin default `requiresApproval`.
#[must_use]
pub fn effective_policy(
    tool_id: &str,
    policies: &[ToolPolicy],
    owner_rank: impl Fn(Owner) -> u8,
    default_requires_approval: bool,
) -> EffectivePolicy {
    match resolve_tool_policy(tool_id, policies, owner_rank) {
        Some(m) => EffectivePolicy {
            action: m.action,
            source: PolicySource::User,
            pattern: Some(m.pattern),
            policy_id: Some(m.policy_id),
        },
        None => EffectivePolicy::plugin_default(default_requires_approval),
    }
}

/// Resolution when rows are already sorted and may omit owner (flat tests).
#[must_use]
pub fn effective_policy_from_sorted(
    tool_id: &str,
    rows: &[(PolicyId, String, PolicyAction, Option<Owner>)],
    default_requires_approval: bool,
) -> EffectivePolicy {
    let mut first_by_owner: Vec<(String, EffectivePolicy)> = Vec::new();
    for (id, pattern, action, owner) in rows {
        let owner_key = owner.map_or_else(|| "__flat__".to_owned(), |o| o.as_str().to_owned());
        if first_by_owner.iter().any(|(k, _)| k == &owner_key) {
            continue;
        }
        if match_pattern(pattern, tool_id) {
            first_by_owner.push((
                owner_key,
                EffectivePolicy {
                    action: *action,
                    source: PolicySource::User,
                    pattern: Some(pattern.clone()),
                    policy_id: Some(id.as_str().to_owned()),
                },
            ));
        }
    }
    let mut selected: Option<EffectivePolicy> = None;
    for (_, m) in first_by_owner {
        selected = Some(match selected {
            None => m,
            Some(cur) if m.action.restriction_rank() > cur.action.restriction_rank() => m,
            Some(cur) => cur,
        });
    }
    selected.unwrap_or_else(|| EffectivePolicy::plugin_default(default_requires_approval))
}

#[cfg(test)]
mod tests {
    use super::{
        PolicyAction, PolicyId, PolicyPattern, PolicySource, ToolPolicy, effective_policy,
        effective_policy_from_sorted, is_valid_pattern, match_pattern, pattern_specificity,
        resolve_tool_policy,
    };
    use crate::Owner;

    fn row(
        id: &str,
        pattern: &str,
        action: PolicyAction,
        position: &str,
        owner: Owner,
    ) -> ToolPolicy {
        ToolPolicy {
            id: PolicyId::new(id).unwrap(),
            owner,
            pattern: PolicyPattern::new(pattern).unwrap(),
            action,
            position: position.to_owned(),
        }
    }

    const fn flat_rank(_: Owner) -> u8 {
        0
    }

    const fn owner_rank(owner: Owner) -> u8 {
        owner.outer_rank()
    }

    #[test]
    fn match_exact() {
        assert!(match_pattern("vercel.dns.create", "vercel.dns.create"));
        assert!(!match_pattern("vercel.dns.create", "vercel.dns.delete"));
    }

    #[test]
    fn match_subtree() {
        assert!(match_pattern("vercel.dns.*", "vercel.dns.create"));
        assert!(match_pattern("vercel.dns.*", "vercel.dns.delete"));
        assert!(match_pattern("vercel.dns.*", "vercel.dns.zones.list"));
        assert!(!match_pattern("vercel.dns.*", "vercel.dnstool"));
        assert!(!match_pattern("vercel.dns.*", "vercel.deploy"));
    }

    #[test]
    fn match_plugin_wide() {
        assert!(match_pattern("vercel.*", "vercel.dns.create"));
        assert!(match_pattern("vercel.*", "vercel.deploy"));
        assert!(!match_pattern("vercel.*", "vercelapp.deploy"));
    }

    #[test]
    fn match_universal() {
        assert!(match_pattern("*", "vercel.dns.create"));
        assert!(match_pattern("*", "github.repos.list"));
        assert!(match_pattern("*", "x"));
    }

    #[test]
    fn match_mid_segment() {
        assert!(match_pattern(
            "github.*.*.repos.list",
            "github.org.acme.repos.list"
        ));
        assert!(match_pattern(
            "github.*.*.repos.list",
            "github.user.alice.repos.list"
        ));
        assert!(!match_pattern(
            "github.*.*.repos.list",
            "github.org.acme.repos.delete"
        ));
        assert!(!match_pattern(
            "github.*.*.repos.list",
            "github.acme.repos.list"
        ));
        assert!(match_pattern(
            "github.*.*.repos.*",
            "github.org.acme.repos.list"
        ));
        assert!(match_pattern("github.*.*.repos.*", "github.org.acme.repos"));
        assert!(!match_pattern(
            "github.*.*.repos.*",
            "github.org.acme.deploy"
        ));
        assert!(match_pattern(
            "github.user.alice.repos.*",
            "github.user.alice.repos.list"
        ));
        assert!(!match_pattern(
            "github.user.alice.repos.*",
            "github.user.bob.repos.list"
        ));
    }

    #[test]
    fn valid_patterns() {
        assert!(is_valid_pattern("a"));
        assert!(is_valid_pattern("a.b.c"));
        assert!(is_valid_pattern("a.*"));
        assert!(is_valid_pattern("a.b.*"));
        assert!(is_valid_pattern("a.*.b"));
        assert!(is_valid_pattern("github.*.*.repos.list"));
        assert!(is_valid_pattern("github.*.*.repos.*"));
        assert!(is_valid_pattern("*"));
        assert!(!is_valid_pattern(""));
        assert!(!is_valid_pattern(".a"));
        assert!(!is_valid_pattern("a."));
        assert!(!is_valid_pattern("a..b"));
        assert!(!is_valid_pattern("*.a"));
        assert!(!is_valid_pattern("a*"));
        assert!(!is_valid_pattern("a.b*"));
    }

    #[test]
    fn resolve_no_match() {
        let policies = [row("a", "github.*", PolicyAction::Block, "a0", Owner::Org)];
        assert!(resolve_tool_policy("vercel.dns.create", &policies, flat_rank).is_none());
    }

    #[test]
    fn resolve_first_by_position() {
        let policies = [
            row(
                "a",
                "vercel.dns.create",
                PolicyAction::Approve,
                "a0",
                Owner::Org,
            ),
            row(
                "b",
                "vercel.dns.*",
                PolicyAction::RequireApproval,
                "a1",
                Owner::Org,
            ),
        ];
        let result = resolve_tool_policy("vercel.dns.create", &policies, flat_rank).unwrap();
        assert_eq!(result.action, PolicyAction::Approve);
        assert_eq!(result.pattern, "vercel.dns.create");
        assert_eq!(result.policy_id, "a");
    }

    #[test]
    fn resolve_broader_when_specific_is_below() {
        let policies = [
            row(
                "b",
                "vercel.dns.*",
                PolicyAction::RequireApproval,
                "a0",
                Owner::Org,
            ),
            row(
                "a",
                "vercel.dns.create",
                PolicyAction::Approve,
                "a1",
                Owner::Org,
            ),
        ];
        let result = resolve_tool_policy("vercel.dns.create", &policies, flat_rank).unwrap();
        assert_eq!(result.action, PolicyAction::RequireApproval);
        assert_eq!(result.pattern, "vercel.dns.*");
    }

    #[test]
    fn inner_approve_cannot_weaken_outer_block() {
        let policies = [
            row("outer", "vercel.*", PolicyAction::Block, "a0", Owner::Org),
            row(
                "inner",
                "vercel.dns.create",
                PolicyAction::Approve,
                "a0",
                Owner::User,
            ),
        ];
        let result = resolve_tool_policy("vercel.dns.create", &policies, owner_rank).unwrap();
        assert_eq!(result.action, PolicyAction::Block);
        assert_eq!(result.policy_id, "outer");
    }

    #[test]
    fn inner_can_strengthen_outer_approve() {
        let policies = [
            row("outer", "vercel.*", PolicyAction::Approve, "a0", Owner::Org),
            row(
                "inner",
                "vercel.dns.create",
                PolicyAction::RequireApproval,
                "a0",
                Owner::User,
            ),
        ];
        let result = resolve_tool_policy("vercel.dns.create", &policies, owner_rank).unwrap();
        assert_eq!(result.action, PolicyAction::RequireApproval);
        assert_eq!(result.policy_id, "inner");
    }

    #[test]
    fn tiebreak_by_id() {
        let a = resolve_tool_policy(
            "vercel.dns.create",
            &[
                row("z", "vercel.dns.*", PolicyAction::Block, "a0", Owner::Org),
                row("a", "vercel.dns.*", PolicyAction::Approve, "a0", Owner::Org),
            ],
            flat_rank,
        )
        .unwrap();
        let b = resolve_tool_policy(
            "vercel.dns.create",
            &[
                row("a", "vercel.dns.*", PolicyAction::Approve, "a0", Owner::Org),
                row("z", "vercel.dns.*", PolicyAction::Block, "a0", Owner::Org),
            ],
            flat_rank,
        )
        .unwrap();
        assert_eq!(a.policy_id, "a");
        assert_eq!(b.policy_id, "a");
    }

    #[test]
    fn effective_user_over_plugin_default() {
        let id = PolicyId::new("a").unwrap();
        let result = effective_policy_from_sorted(
            "vercel.dns.create",
            &[(id, "vercel.dns.create".into(), PolicyAction::Approve, None)],
            true,
        );
        assert_eq!(result.action, PolicyAction::Approve);
        assert_eq!(result.source, PolicySource::User);
    }

    #[test]
    fn specificity_scores() {
        assert_eq!(pattern_specificity("*"), 0);
        assert_eq!(pattern_specificity("vercel.*"), 2);
        assert_eq!(pattern_specificity("vercel.dns.*"), 4);
        assert_eq!(pattern_specificity("vercel.dns"), 5);
        assert_eq!(pattern_specificity("vercel.dns.create"), 7);
    }

    #[test]
    fn effective_falls_back_to_plugin() {
        let result = effective_policy("x.y", &[], flat_rank, true);
        assert_eq!(result.action, PolicyAction::RequireApproval);
        assert_eq!(result.source, PolicySource::PluginDefault);
    }
}
