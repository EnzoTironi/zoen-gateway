//! Named toolkit: a scoped MCP surface (`/mcp/toolkits/:slug`).

use serde::{Deserialize, Serialize};

use crate::{Owner, PolicyAction};

/// One toolkit partition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Toolkit {
    /// Stable id.
    pub id: String,
    /// Org or user.
    pub owner: Owner,
    /// URL slug (`/mcp/toolkits/{slug}`).
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Connection-address glob patterns this toolkit exposes.
    #[serde(default)]
    pub connections: Vec<String>,
    /// Extra policy rows applied inside the toolkit (`pattern` + `action`).
    #[serde(default)]
    pub policies: Vec<ToolkitPolicy>,
}

/// Toolkit-local policy overlay.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolkitPolicy {
    /// Policy id.
    pub id: String,
    /// Tool pattern.
    pub pattern: String,
    /// Action.
    pub action: PolicyAction,
}

impl Toolkit {
    /// Whether `address` (connection or tool) is in this toolkit.
    #[must_use]
    pub fn allows_address(&self, address: &str) -> bool {
        if self.connections.is_empty() {
            return false;
        }
        self.connections.iter().any(|pat| glob_match(pat, address))
    }
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let pat = pattern.trim();
    if pat == "*" {
        return true;
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    pat == value || value.starts_with(&format!("{pat}.")) || value.starts_with(&format!("{pat}/"))
}

/// Collection names for [`crate::CatalogStore::put_kv`].
pub const KV_TOOLKITS: &str = "toolkits";
/// OAuth client registrations.
pub const KV_OAUTH_CLIENTS: &str = "oauth_clients";
/// In-flight OAuth sessions (PKCE verifier).
pub const KV_OAUTH_SESSIONS: &str = "oauth_sessions";
/// Host principals seen by this tenant (`subject.external_id`).
pub const KV_SUBJECTS: &str = "subjects";
/// Per-tool session-scoped approvals (`persist=session`).
pub const KV_SESSION_APPROVALS: &str = "session_approvals";
/// Teams (`slug` → name, owner, overflow opt-out).
pub const KV_ORGS: &str = "orgs";
/// Org memberships (`{org}:{subject}`).
pub const KV_MEMBERSHIPS: &str = "memberships";
/// Invite codes (`code` → org, role, email).
pub const KV_INVITES: &str = "invites";
/// Uploaded `SKILL.md` bundles.
pub const KV_SKILLS: &str = "skills";
/// Named artifacts (notes, outputs, generated UI).
pub const KV_ARTIFACTS: &str = "artifacts";
/// Enrich Arena votes (`capability` → winner id + counts).
pub const KV_ARENA_VOTES: &str = "arena_votes";
/// Local single-user sentinel (original host `subject` partition).
pub const LOCAL_SUBJECT: &str = "local";
