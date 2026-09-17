//! Treg-shaped YAML ingest. Extra fields are ignored; meta files are skipped.

use std::path::Path;

use serde_json::{Value as Json, json};
use serde_yaml::Value;

use crate::{Access, CatalogError, Endpoint};

const SKIP_FILES: &[&str] = &[
    "adapters.yaml",
    "adapters.yml",
    "aliases.yaml",
    "aliases.yml",
    "capabilities.yaml",
    "capabilities.yml",
    "contracts.yaml",
    "contracts.yml",
];

/// Directories to merge after the bundled seed (`EXECUTOR_CATALOG_DIR`, `{data}/catalog`).
#[must_use]
pub fn catalog_search_dirs(data_dir: Option<&Path>) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(raw) = std::env::var("EXECUTOR_CATALOG_DIR") {
        for part in raw.split(':') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                dirs.push(std::path::PathBuf::from(trimmed));
            }
        }
    }
    if let Some(data) = data_dir {
        dirs.push(data.join("catalog"));
    }
    dirs
}

fn include_extended() -> bool {
    matches!(
        std::env::var("EXECUTOR_CATALOG_INCLUDE_EXTENDED")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

fn skip_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    if SKIP_FILES.contains(&lower.as_str()) {
        return true;
    }
    !include_extended() && lower.contains(".extended.")
}

/// Load every `*.yaml` / `*.yml` in `dir`. Missing directory is empty.
///
/// # Errors
///
/// A readable file that is not YAML.
pub fn load_yaml_dir(dir: &Path) -> Result<Vec<Endpoint>, CatalogError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut endpoints = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if ext != "yaml" && ext != "yml" {
            continue;
        }
        if skip_file(&path) {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
        match parse_treg_yaml(&text) {
            Ok(more) => endpoints.extend(more),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "catalog yaml skipped");
            }
        }
    }
    Ok(endpoints)
}

/// Parse one Treg provider document (`provider` + `endpoints`) or a bare list.
///
/// # Errors
///
/// YAML syntax.
pub fn parse_treg_yaml(text: &str) -> Result<Vec<Endpoint>, CatalogError> {
    let doc: Value =
        serde_yaml::from_str(text).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
    Ok(parse_treg_value(&doc))
}

fn parse_treg_value(doc: &Value) -> Vec<Endpoint> {
    if let Some(seq) = doc.as_sequence() {
        return seq
            .iter()
            .filter_map(|item| endpoint_from_yaml(item, "unknown"))
            .collect();
    }
    let provider = doc
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let Some(endpoints) = doc.get("endpoints").and_then(Value::as_sequence) else {
        return Vec::new();
    };
    endpoints
        .iter()
        .filter_map(|item| endpoint_from_yaml(item, provider))
        .collect()
}

fn endpoint_from_yaml(value: &Value, provider: &str) -> Option<Endpoint> {
    let id = value.get("id").and_then(Value::as_str)?.to_owned();
    if id.is_empty() {
        return None;
    }
    let capability = value
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_owned();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_owned();
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_owned();
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_owned();
    let strict_query = value
        .get("strict_query")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let query = query_schema(value.get("input").and_then(|i| i.get("queryParams")));
    let mut jobs = string_list(value.get("jobs"));
    if jobs.is_empty() {
        jobs.push(capability.replace('.', " "));
        jobs.push(name.to_ascii_lowercase());
        jobs.push(provider.to_owned());
    }
    let cost_micro = treg_cost_to_micro(value.get("cost"));
    let access = if cost_micro.is_some() {
        Access::Platform
    } else {
        Access::OwnKeyOnly
    };
    Some(Endpoint {
        id,
        capability,
        provider: provider.to_owned(),
        name,
        summary,
        jobs,
        method,
        path,
        base_url: format!("mock://{provider}"),
        access,
        cost_micro,
        query,
        routed_child: None,
        strict_query,
    })
}

fn query_schema(raw: Option<&Value>) -> Json {
    let Some(map) = raw.and_then(Value::as_mapping) else {
        return Json::Object(serde_json::Map::new());
    };
    let mut out = serde_json::Map::new();
    for (key, spec) in map {
        let Some(name) = key.as_str() else {
            continue;
        };
        let required = spec
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let ty = spec.get("type").and_then(Value::as_str).unwrap_or("string");
        let mut field = serde_json::Map::new();
        field.insert("type".into(), json!(ty));
        field.insert("required".into(), json!(required));
        if let Some(example) = spec.get("example") {
            field.insert("example".into(), yaml_to_json_string(example));
        }
        if let Some(enums) = spec.get("enum").and_then(Value::as_sequence) {
            let values: Vec<String> = enums
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            field.insert("enum".into(), json!(values));
        }
        out.insert(name.to_owned(), Json::Object(field));
    }
    Json::Object(out)
}

fn yaml_to_json_string(value: &Value) -> Json {
    if let Some(s) = value.as_str() {
        return json!(s);
    }
    if let Some(n) = value.as_i64() {
        return json!(n.to_string());
    }
    if let Some(n) = value.as_f64() {
        return json!(n.to_string());
    }
    json!(value.as_bool().map_or(String::new(), |b| b.to_string()))
}

fn string_list(raw: Option<&Value>) -> Vec<String> {
    raw.and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn treg_cost_to_micro(cost: Option<&Value>) -> Option<i64> {
    let cost = cost?;
    let value = cost.get("value")?.as_f64()?;
    if value < 0.0 {
        return None;
    }
    let currency = cost
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("usd");
    if !currency.eq_ignore_ascii_case("usd") {
        // Credits / vendor units are not micro-USD; leave unpublished.
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    Some((value * 1_000_000.0).round() as i64)
}
