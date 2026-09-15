//! Ranked tool discovery used by `tools.search` (ported from executor `tool-invoker.ts`).

use serde::{Deserialize, Serialize};

/// One page of search/list results.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage<T> {
    /// Hits on this page.
    pub items: Vec<T>,
    /// Total matches before paging.
    pub total: u32,
    /// More pages exist.
    pub has_more: bool,
    /// Pass as `offset` for the next page.
    pub next_offset: Option<u32>,
}

impl<T> SearchPage<T> {
    /// Slice `items` at `offset`/`limit`.
    #[must_use]
    pub fn paginate(items: Vec<T>, offset: u32, limit: u32) -> Self {
        let total = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(items.len());
        let take = usize::try_from(limit.max(1)).unwrap_or(1);
        let page: Vec<T> = items.into_iter().skip(start).take(take).collect();
        let taken = u32::try_from(page.len()).unwrap_or(0);
        let has_more = offset.saturating_add(taken) < total;
        Self {
            items: page,
            total,
            has_more,
            next_offset: has_more.then_some(offset.saturating_add(taken)),
        }
    }
}

/// One ranked tool hit (`tools.search` item).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiscovery {
    /// Sandbox path under `tools.` (no `tools.` prefix).
    pub path: String,
    /// Tool leaf name.
    pub name: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Integration slug.
    pub integration: String,
    /// Rank score (0 when enumerating a namespace).
    pub score: i32,
}

/// Arguments to `tools.search`.
#[derive(Clone, Debug, Default)]
pub struct SearchArgs {
    /// Free-text query. Empty + namespace enumerates that integration.
    pub query: String,
    /// Integration / path token prefix.
    pub namespace: Option<String>,
    /// Page size (default 12).
    pub limit: u32,
    /// Page start.
    pub offset: u32,
}

impl SearchArgs {
    /// Read from a JSON object (`query`, `namespace`, `limit`, `offset`).
    #[must_use]
    pub fn from_value(value: &serde_json::Value) -> Self {
        let obj = value.as_object();
        let query = obj
            .and_then(|m| m.get("query"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        let namespace = obj
            .and_then(|m| m.get("namespace").or_else(|| m.get("integration")))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .filter(|s| !s.is_empty());
        let limit = obj
            .and_then(|m| m.get("limit"))
            .and_then(serde_json::Value::as_u64)
            .map_or(12, |n| u32::try_from(n).unwrap_or(12));
        let offset = obj
            .and_then(|m| m.get("offset"))
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |n| u32::try_from(n).unwrap_or(0));
        Self {
            query,
            namespace,
            limit: limit.clamp(1, 100),
            offset,
        }
    }
}

/// Catalog fields needed to rank a tool.
pub struct SearchableTool<'a> {
    /// Sandbox path.
    pub path: &'a str,
    /// Leaf name.
    pub name: &'a str,
    /// Description.
    pub description: &'a str,
    /// Integration slug.
    pub integration: &'a str,
}

const W_PATH: i32 = 12;
const W_INTEGRATION: i32 = 8;
const W_NAME: i32 = 10;
const W_DESCRIPTION: i32 = 5;

/// Split camelCase / punctuation into lowercase tokens.
#[must_use]
pub fn tokenize_search_text(value: &str) -> Vec<String> {
    normalize_search_text(value)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Lowercase, split camelCase, collapse separators.
#[must_use]
pub fn normalize_search_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    let mut prev_alnum_lower = false;
    for ch in value.chars() {
        if ch.is_ascii_uppercase() && prev_alnum_lower {
            out.push(' ');
        }
        if ch == '_' || ch == '.' || ch == '/' || ch == ':' || ch == '-' {
            out.push(' ');
            prev_alnum_lower = false;
            continue;
        }
        out.push(ch.to_ascii_lowercase());
        prev_alnum_lower = ch.is_ascii_alphanumeric() && !ch.is_ascii_uppercase();
        if ch.is_ascii_uppercase() {
            prev_alnum_lower = true;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rank `tools` for `args`. Empty query with no namespace yields an empty page.
#[must_use]
pub fn search_tools(tools: &[SearchableTool<'_>], args: &SearchArgs) -> SearchPage<ToolDiscovery> {
    let empty_query = normalize_search_text(&args.query).is_empty();
    let has_ns = args
        .namespace
        .as_deref()
        .is_some_and(|n| !normalize_search_text(n).is_empty());
    if empty_query && !has_ns {
        return SearchPage {
            items: Vec::new(),
            total: 0,
            has_more: false,
            next_offset: None,
        };
    }
    let ranked: Vec<ToolDiscovery> = if empty_query {
        let ns = args.namespace.as_deref().unwrap_or("").trim();
        let mut hits: Vec<ToolDiscovery> = tools
            .iter()
            .filter(|t| t.integration == ns)
            .map(|t| discovery(t, 0))
            .collect();
        hits.sort_by(|a, b| a.path.cmp(&b.path));
        hits
    } else {
        let mut hits: Vec<ToolDiscovery> = tools
            .iter()
            .filter(|t| matches_namespace(t, args.namespace.as_deref()))
            .filter_map(|t| score_tool_match(t, &args.query))
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        hits
    };
    SearchPage::paginate(ranked, args.offset, args.limit)
}

fn discovery(tool: &SearchableTool<'_>, score: i32) -> ToolDiscovery {
    ToolDiscovery {
        path: tool.path.to_owned(),
        name: tool.name.to_owned(),
        description: (!tool.description.is_empty()).then(|| tool.description.to_owned()),
        integration: tool.integration.to_owned(),
        score,
    }
}

fn matches_namespace(tool: &SearchableTool<'_>, namespace: Option<&str>) -> bool {
    let Some(namespace) = namespace.filter(|n| !normalize_search_text(n).is_empty()) else {
        return true;
    };
    let ns = tokenize_search_text(namespace);
    if ns.is_empty() {
        return true;
    }
    is_prefix_match(&tokenize_search_text(tool.integration), &ns)
        || is_prefix_match(&tokenize_search_text(tool.path), &ns)
}

fn is_prefix_match(tokens: &[String], prefix: &[String]) -> bool {
    prefix
        .iter()
        .enumerate()
        .all(|(i, tok)| tokens.get(i) == Some(tok))
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn score_tool_match(tool: &SearchableTool<'_>, query: &str) -> Option<ToolDiscovery> {
    let normalized = normalize_search_text(query);
    let query_tokens = tokenize_search_text(query);
    if normalized.is_empty() || query_tokens.is_empty() {
        return None;
    }
    let path = prepared(tool.path);
    let integration = prepared(tool.integration);
    let name = prepared(tool.name);
    let description = prepared(tool.description);
    let fields = [
        score_field(&normalized, &query_tokens, &path, W_PATH),
        score_field(&normalized, &query_tokens, &integration, W_INTEGRATION),
        score_field(&normalized, &query_tokens, &name, W_NAME),
        score_field(&normalized, &query_tokens, &description, W_DESCRIPTION),
    ];
    let mut score = 0;
    let mut matched = std::collections::BTreeSet::new();
    let mut exact_phrase = false;
    for field in fields {
        score += field.0;
        exact_phrase |= field.1;
        matched.extend(field.2);
    }
    if matched.is_empty() {
        return None;
    }
    let coverage = matched.len() as f64 / query_tokens.len() as f64;
    let min_cov = if query_tokens.len() <= 2 { 1.0 } else { 0.6 };
    if coverage < min_cov && !exact_phrase {
        return None;
    }
    if (coverage - 1.0).abs() < f64::EPSILON {
        score += 25;
    } else {
        score += (coverage * 10.0).round() as i32;
    }
    if path.1.first() == query_tokens.first() || name.1.first() == query_tokens.first() {
        score += 8;
    }
    if normalize_search_text(tool.path) == normalized
        || normalize_search_text(tool.name) == normalized
    {
        score += 20;
    }
    Some(discovery(tool, score))
}

fn prepared(value: &str) -> (String, Vec<String>) {
    (normalize_search_text(value), tokenize_search_text(value))
}

fn score_field(
    query: &str,
    query_tokens: &[String],
    field: &(String, Vec<String>),
    weight: i32,
) -> (i32, bool, Vec<String>) {
    if field.0.is_empty() {
        return (0, false, Vec::new());
    }
    let mut score = 0;
    let mut matched = Vec::new();
    let exact_phrase = !query.is_empty() && field.0.contains(query);
    if !query.is_empty() {
        if field.0 == query {
            score += weight * 14;
        } else if field.0.starts_with(query) {
            score += weight * 9;
        } else if exact_phrase {
            score += weight * 6;
        }
    }
    for token in query_tokens {
        if field.1.iter().any(|c| c == token) {
            score += weight * 4;
            matched.push(token.clone());
            continue;
        }
        if field
            .1
            .iter()
            .any(|c| c.starts_with(token) || token.starts_with(c))
        {
            score += weight * 2;
            matched.push(token.clone());
            continue;
        }
        if field.0.contains(token) {
            score += weight;
            matched.push(token.clone());
        }
    }
    (score, exact_phrase, matched)
}

#[cfg(test)]
mod tests {
    use super::{SearchArgs, SearchableTool, search_tools};

    #[test]
    fn empty_query_without_namespace_is_empty() {
        let tools = [SearchableTool {
            path: "github.org.main.issues",
            name: "issues",
            description: "list issues",
            integration: "github",
        }];
        let page = search_tools(&tools, &SearchArgs::default());
        assert!(page.items.is_empty());
    }

    #[test]
    fn namespace_enumeration_is_path_sorted() {
        let tools = [
            SearchableTool {
                path: "github.org.main.z",
                name: "z",
                description: "",
                integration: "github",
            },
            SearchableTool {
                path: "github.org.main.a",
                name: "a",
                description: "",
                integration: "github",
            },
        ];
        let page = search_tools(
            &tools,
            &SearchArgs {
                namespace: Some("github".into()),
                limit: 12,
                ..SearchArgs::default()
            },
        );
        assert_eq!(page.items[0].path, "github.org.main.a");
        assert_eq!(page.items[0].score, 0);
    }

    #[test]
    fn ranked_query_prefers_path_hits() {
        let tools = [
            SearchableTool {
                path: "crm.org.main.create_contact",
                name: "createContact",
                description: "make a person",
                integration: "crm",
            },
            SearchableTool {
                path: "github.org.main.issues_create",
                name: "createIssue",
                description: "open a github issue",
                integration: "github",
            },
        ];
        let page = search_tools(
            &tools,
            &SearchArgs {
                query: "github issues".into(),
                limit: 5,
                ..SearchArgs::default()
            },
        );
        assert_eq!(page.items[0].integration, "github");
        assert!(page.items[0].score > 0);
    }
}
