//! CLI tool-path browsing (`executor call github --help`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};
use thiserror::Error;

/// Path grammar / JSON errors.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ToolPathError {
    /// Empty path.
    #[error("tool path must include at least one segment")]
    Empty,
    /// Illegal characters in a segment.
    #[error("tool path segments must contain only letters, numbers, '.', '_' or '-'")]
    BadSegment,
    /// Arguments were not a JSON object.
    #[error("tool arguments must decode to a JSON object")]
    ArgsNotObject,
    /// JSON parse failed.
    #[error("invalid JSON arguments: {0}")]
    InvalidJson(String),
    /// `@` without a path.
    #[error("tool input '@' requires a file path, e.g. `@./input.json`")]
    EmptyFileRef,
    /// Cannot read `@file`.
    #[error("cannot read tool input file '{path}': {io}")]
    FileRead {
        /// Path.
        path: String,
        /// IO error.
        io: String,
    },
    /// File did not start with `{`.
    #[error("tool input file '{0}' must contain a JSON object starting with '{{'")]
    FileNotObject(String),
    /// Legacy flags.
    #[error(
        "tool invocation no longer accepts flags. Use: executor call <path...> '{{...json...}}'"
    )]
    Flags,
}

fn is_token(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn to_segments(parts: &[impl AsRef<str>]) -> Vec<String> {
    parts
        .iter()
        .flat_map(|part| part.as_ref().split('.'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Join path parts into a dotted tool path.
///
/// # Errors
///
/// Empty or illegal segments.
pub fn build_tool_path(parts: &[impl AsRef<str>]) -> Result<String, ToolPathError> {
    let segments = to_segments(parts);
    if segments.is_empty() {
        return Err(ToolPathError::Empty);
    }
    if segments.iter().any(|s| !is_token(s)) {
        return Err(ToolPathError::BadSegment);
    }
    Ok(segments.join("."))
}

/// One child under a path prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolPathChild {
    /// Next segment.
    pub segment: String,
    /// A tool exists at prefix+segment.
    pub invokable: bool,
    /// Deeper tools exist.
    pub has_children: bool,
    /// How many tools sit under this child.
    pub tool_count: usize,
}

/// Result of browsing a prefix against a catalog of paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolPathInspection {
    /// Normalized prefix.
    pub prefix_segments: Vec<String>,
    /// Exact tool path when the prefix is itself a tool.
    pub exact_path: Option<String>,
    /// Tools under the prefix.
    pub matching_tool_count: usize,
    /// Child segments.
    pub children: Vec<ToolPathChild>,
}

/// Browse `tool_paths` under `raw_prefix_parts`.
#[must_use]
pub fn inspect_tool_path(tool_paths: &[String], raw_prefix_parts: &[String]) -> ToolPathInspection {
    let prefix_segments = if raw_prefix_parts.is_empty() {
        Vec::new()
    } else {
        to_segments(raw_prefix_parts)
    };
    let mut children: BTreeMap<String, (bool, bool, usize)> = BTreeMap::new();
    let mut exact_path = None;
    let mut matching_tool_count = 0usize;

    for path in tool_paths {
        let segments = to_segments(&[path.as_str()]);
        if segments.is_empty() || !is_prefix(&prefix_segments, &segments) {
            continue;
        }
        matching_tool_count += 1;
        if segments.len() == prefix_segments.len() {
            if exact_path.is_none() {
                exact_path = Some(segments.join("."));
            }
            continue;
        }
        let Some(child) = segments.get(prefix_segments.len()) else {
            continue;
        };
        let entry = children.entry(child.clone()).or_insert((false, false, 0));
        entry.0 |= segments.len() == prefix_segments.len() + 1;
        entry.1 |= segments.len() > prefix_segments.len() + 1;
        entry.2 += 1;
    }

    let children = children
        .into_iter()
        .map(
            |(segment, (invokable, has_children, tool_count))| ToolPathChild {
                segment,
                invokable,
                has_children,
                tool_count,
            },
        )
        .collect();

    ToolPathInspection {
        prefix_segments,
        exact_path,
        matching_tool_count,
        children,
    }
}

fn is_prefix(prefix: &[String], path: &[String]) -> bool {
    prefix.len() <= path.len() && prefix.iter().zip(path.iter()).all(|(a, b)| a == b)
}

/// Parsed `executor call` invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    /// Dotted path.
    pub path: String,
    /// JSON object args.
    pub args: Map<String, Value>,
}

/// Split path parts and a trailing JSON object / `@file`.
///
/// # Errors
///
/// See [`ToolPathError`].
pub fn resolve_invocation(raw_path_parts: &[String]) -> Result<Invocation, ToolPathError> {
    let last = raw_path_parts.last().map(|s| s.trim().to_owned());
    let is_file = last.as_deref().is_some_and(|s| s.starts_with('@'));
    if is_file && last.as_deref() == Some("@") {
        return Err(ToolPathError::EmptyFileRef);
    }
    let json_text = if is_file {
        let path = last.as_ref().map_or("", |s| &s[1..]);
        if path.is_empty() {
            return Err(ToolPathError::EmptyFileRef);
        }
        fs::read_to_string(Path::new(path))
            .map_err(|e| ToolPathError::FileRead {
                path: path.to_owned(),
                io: e.to_string(),
            })?
            .trim()
            .to_owned()
    } else {
        last.clone().unwrap_or_default()
    };
    let has_json = json_text.starts_with('{');
    if is_file && !has_json {
        let path = last.as_ref().map_or("", |s| &s[1..]).to_owned();
        return Err(ToolPathError::FileNotObject(path));
    }
    let path_parts: Vec<String> = if is_file || has_json {
        raw_path_parts[..raw_path_parts.len().saturating_sub(1)].to_vec()
    } else {
        raw_path_parts.to_vec()
    };
    if path_parts.iter().any(|part| part.trim().starts_with('-')) {
        return Err(ToolPathError::Flags);
    }
    let args = if has_json {
        parse_json_object(&json_text)?
    } else {
        Map::new()
    };
    let path = build_tool_path(&path_parts)?;
    Ok(Invocation { path, args })
}

/// Compile a single `call` into the code-mode subset (`return await tools[path](args)`).
#[must_use]
pub fn compile_call(path: &str, args: &Value) -> String {
    let path = serde_json::to_string(path).unwrap_or_else(|_| "\"\"".into());
    let args = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
    format!("return await tools[{path}]({args});")
}

fn parse_json_object(raw: &str) -> Result<Map<String, Value>, ToolPathError> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|e| ToolPathError::InvalidJson(e.to_string()))?;
    parsed
        .as_object()
        .cloned()
        .ok_or(ToolPathError::ArgsNotObject)
}

#[cfg(test)]
mod tests {
    use super::{
        ToolPathError, build_tool_path, compile_call, inspect_tool_path, resolve_invocation,
    };
    use serde_json::json;

    #[test]
    fn builds_and_inspects() {
        let path = build_tool_path(&["github", "issues.create"]).unwrap();
        assert_eq!(path, "github.issues.create");
        let paths = vec![
            "tools.github.org.work.issues.create".into(),
            "tools.github.org.work.issues.list".into(),
            "executor.openapi.addSpec".into(),
        ];
        let inspection = inspect_tool_path(&paths, &["tools".into(), "github".into()]);
        assert_eq!(inspection.matching_tool_count, 2);
        assert_eq!(inspection.children.len(), 1);
        assert_eq!(inspection.children[0].segment, "org");
    }

    #[test]
    fn invocation_json() {
        let inv = resolve_invocation(&[
            "github".into(),
            "issues".into(),
            "create".into(),
            r#"{"title":"Hi"}"#.into(),
        ])
        .unwrap();
        assert_eq!(inv.path, "github.issues.create");
        assert_eq!(inv.args["title"], "Hi");
    }

    #[test]
    fn rejects_flags() {
        let err = resolve_invocation(&["github".into(), "--help".into()]).unwrap_err();
        assert_eq!(err, ToolPathError::Flags);
    }

    #[test]
    fn rejects_bad_segment() {
        assert!(build_tool_path(&["bad*"]).is_err());
    }

    #[test]
    fn compiles_call_to_bracket_access() {
        let src = compile_call("echo.org.work.ping", &json!({"n": 1}));
        assert_eq!(src, r#"return await tools["echo.org.work.ping"]({"n":1});"#);
    }
}
