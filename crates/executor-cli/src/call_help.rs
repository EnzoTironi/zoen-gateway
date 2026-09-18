//! `executor call --help` namespace browse (original `runCallHelp`).

use executor_core::{ToolPathChild, inspect_tool_path};
use serde_json::Value;

use crate::daemon;

/// Browse the catalog under `prefix` (`--match` / `--limit` optional).
///
/// # Errors
///
/// HTTP, or no matching tools (exit-worthy).
pub async fn run(
    origin: &str,
    token: Option<&str>,
    prefix: &[String],
    match_query: Option<&str>,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = daemon::get_json(origin, "/api/tools", token).await?;
    let tools = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let paths: Vec<String> = tools
        .iter()
        .filter_map(|t| {
            t.get("address")
                .and_then(Value::as_str)
                .or_else(|| t.get("path").and_then(Value::as_str))
                .map(ToOwned::to_owned)
        })
        .collect();
    let inspection = inspect_with_tools_prefix(&paths, prefix);
    if inspection.matching_tool_count == 0 {
        let typed = inspection.prefix_segments.join(".");
        if typed.is_empty() {
            eprintln!("No tools are currently registered.");
        } else {
            eprintln!("No tool path starts with \"{typed}\".");
        }
        let mut fallback = inspect_tool_path(&paths, &[]);
        let mut mismatch: Option<String> = None;
        for depth in (0..inspection.prefix_segments.len()).rev() {
            let candidate_prefix = &inspection.prefix_segments[..depth];
            let candidate = inspect_tool_path(&paths, candidate_prefix);
            if candidate.matching_tool_count > 0 {
                fallback = candidate;
                mismatch = inspection.prefix_segments.get(depth).cloned();
                break;
            }
        }
        let query = match_query.or(mismatch.as_deref());
        let children = filter_children(&fallback.children, query, limit);
        print_browse(&fallback.prefix_segments, &children, query, limit, None);
        return Err("no matching tool path".into());
    }

    let exact = inspection.exact_path.as_ref().and_then(|addr| {
        tools.iter().find(|t| {
            t.get("address").and_then(Value::as_str) == Some(addr.as_str())
                || t.get("path").and_then(Value::as_str) == Some(addr.as_str())
        })
    });

    if inspection.children.is_empty()
        && let Some(tool) = exact
    {
        print_leaf(origin, token, tool).await?;
        return Ok(());
    }

    let children = filter_children(&inspection.children, match_query, limit);
    let exact_line = exact.map(|t| {
        format!(
            "{}\t{}",
            t.get("address")
                .and_then(Value::as_str)
                .or_else(|| t.get("path").and_then(Value::as_str))
                .unwrap_or(""),
            t.get("description").and_then(Value::as_str).unwrap_or("")
        )
    });
    print_browse(
        &inspection.prefix_segments,
        &children,
        match_query,
        limit,
        exact_line.as_deref(),
    );
    Ok(())
}

fn filter_children(
    children: &[ToolPathChild],
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<ToolPathChild> {
    let q = query.map(str::to_ascii_lowercase);
    let mut out: Vec<ToolPathChild> = children
        .iter()
        .filter(|c| {
            q.as_ref()
                .is_none_or(|needle| c.segment.to_ascii_lowercase().contains(needle))
        })
        .cloned()
        .collect();
    if let Some(n) = limit {
        out.truncate(n);
    }
    out
}

fn print_browse(
    prefix: &[String],
    children: &[ToolPathChild],
    query: Option<&str>,
    limit: Option<usize>,
    exact: Option<&str>,
) {
    println!("Usage: executor call [PATH...] '{{json}}'");
    println!("Browse: executor call --help [PATH...] [--match TEXT] [--limit N]");
    let shown = if prefix.is_empty() {
        "(root)".to_owned()
    } else {
        prefix.join(".")
    };
    println!("prefix: {shown}");
    if let Some(q) = query {
        println!("match: {q}");
    }
    if let Some(n) = limit {
        println!("limit: {n}");
    }
    if let Some(exact) = exact {
        println!("exact:\t{exact}");
    }
    if children.is_empty() {
        println!("(no children)");
        return;
    }
    for child in children {
        let kind = match (child.invokable, child.has_children) {
            (true, true) => "tool+",
            (true, false) => "tool",
            (false, true) => "ns",
            (false, false) => "leaf",
        };
        println!("{}\t{kind}\t{} tools", child.segment, child.tool_count);
    }
}

async fn print_leaf(
    origin: &str,
    token: Option<&str>,
    tool: &Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = tool
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| tool.get("address").and_then(Value::as_str))
        .unwrap_or("");
    let desc = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    println!("{path}");
    println!("{desc}");
    let q = crate::urlencoding_query(path);
    if let Ok(shape) =
        daemon::get_json(origin, &format!("/api/tools/describe?path={q}"), token).await
    {
        println!("{}", serde_json::to_string_pretty(&shape)?);
    }
    Ok(())
}

fn inspect_with_tools_prefix(
    paths: &[String],
    prefix: &[String],
) -> executor_core::ToolPathInspection {
    let first = inspect_tool_path(paths, prefix);
    if first.matching_tool_count > 0 || prefix.is_empty() {
        return first;
    }
    if prefix.first().is_some_and(|s| s == "tools") {
        return first;
    }
    let mut with_tools = vec!["tools".to_owned()];
    with_tools.extend(prefix.iter().cloned());
    let second = inspect_tool_path(paths, &with_tools);
    if second.matching_tool_count > 0 {
        second
    } else {
        first
    }
}
