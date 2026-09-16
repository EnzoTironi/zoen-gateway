//! `executor.jsonc` — Rust-native plugin/integration config (not jiti factories).

use std::path::{Path, PathBuf};

use executor_core::{ExecuteOptions, Outcome};
use executor_engine::Executor;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One plugin install entry (`package` + options).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PluginConfig {
    /// npm-style package name or a first-party kind (`openapi`, `graphql`, `mcp`).
    pub package: String,
    /// Plugin options.
    #[serde(default)]
    pub options: Value,
}

/// One catalog integration declared in jsonc.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IntegrationConfig {
    /// `openapi` | `graphql` | `mcp`.
    pub kind: String,
    /// `OpenAPI` spec URL or document.
    #[serde(default)]
    pub spec: Option<Value>,
    /// Spec fetch URL when `spec` is omitted.
    #[serde(default)]
    pub spec_url: Option<String>,
    /// `OpenAPI` `servers[0].url` override.
    #[serde(default, alias = "baseUrl")]
    pub base_url: Option<String>,
    /// GraphQL HTTP endpoint.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Catalog slug.
    #[serde(default)]
    pub slug: Option<String>,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// MCP stdio command.
    #[serde(default)]
    pub command: Option<String>,
    /// MCP remote URL.
    #[serde(default)]
    pub url: Option<String>,
}

/// Top-level `executor.jsonc`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ExecutorFileConfig {
    /// Workspace label.
    #[serde(default)]
    pub name: Option<String>,
    /// Plugin installs (`package` + options).
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
    /// First-party integrations to register at boot.
    #[serde(default)]
    pub integrations: Vec<IntegrationConfig>,
}

/// Strip `//` and `/* */` comments outside strings.
#[must_use]
pub fn strip_jsonc(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut out = String::with_capacity(source.len());
    let mut in_str = false;
    let mut quote = '\0';
    let mut escape = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == quote {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            in_str = true;
            quote = c;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = i.saturating_add(2);
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Parse jsonc text.
///
/// # Errors
///
/// JSON.
pub fn parse_jsonc(source: &str) -> Result<ExecutorFileConfig, serde_json::Error> {
    serde_json::from_str(&strip_jsonc(source))
}

/// Load `executor.jsonc` from cwd, then `data_dir`.
#[must_use]
pub fn load_jsonc(data_dir: Option<&Path>) -> Option<(PathBuf, ExecutorFileConfig)> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("executor.jsonc"));
    }
    if let Some(dir) = data_dir {
        candidates.push(dir.join("executor.jsonc"));
    }
    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(cfg) = parse_jsonc(&text)
        {
            return Some((path, cfg));
        }
    }
    None
}

fn first_party_kind(package: &str) -> Option<&'static str> {
    let tail = package.rsplit('/').next().unwrap_or(package);
    match tail {
        "openapi" | "plugin-openapi" => Some("openapi"),
        "graphql" | "plugin-graphql" => Some("graphql"),
        "mcp" | "plugin-mcp" => Some("mcp"),
        _ => None,
    }
}

/// Apply jsonc integrations by invoking first-party static tools.
///
/// # Errors
///
/// Execute failures.
pub async fn apply_config(
    exec: &Executor,
    cfg: &ExecutorFileConfig,
) -> Result<(), executor_core::ExecutorError> {
    let yes = ExecuteOptions {
        auto_approve: true,
        ..ExecuteOptions::default()
    };
    for plugin in &cfg.plugins {
        if let Some(kind) = first_party_kind(&plugin.package)
            && let Some(integ) = plugin.options.get("integrations").and_then(Value::as_array)
        {
            for row in integ {
                apply_kind(exec, kind, row, &yes).await?;
            }
        }
    }
    for integ in &cfg.integrations {
        let mut body = serde_json::to_value(integ).unwrap_or(Value::Null);
        if let Some(obj) = body.as_object_mut() {
            if let Some(url) = obj.remove("spec_url") {
                obj.entry("specUrl").or_insert(url);
            }
            if let Some(url) = obj.remove("base_url") {
                obj.entry("baseUrl").or_insert(url);
            }
        }
        apply_kind(exec, &integ.kind, &body, &yes).await?;
    }
    Ok(())
}

async fn apply_kind(
    exec: &Executor,
    kind: &str,
    body: &Value,
    yes: &ExecuteOptions,
) -> Result<(), executor_core::ExecutorError> {
    let path = match kind {
        "openapi" => "executor.openapi.addSpec",
        "graphql" => "executor.graphql.addIntegration",
        "mcp" => "executor.mcp.addServer",
        _ => return Ok(()),
    };
    match exec.execute(path, body.clone(), yes.clone()).await? {
        Outcome::Completed { .. } | Outcome::Paused { .. } => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_jsonc, strip_jsonc};

    #[test]
    fn strips_line_and_block_comments() {
        let src = r#"{
          // name
          "name": "demo",
          /* plugins */
          "plugins": []
        }"#;
        let cfg = parse_jsonc(&strip_jsonc(src)).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("demo"));
    }
}
