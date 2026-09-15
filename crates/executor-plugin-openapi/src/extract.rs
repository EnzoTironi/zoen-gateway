//! Spec parse, operation extract, `group.leaf` tool paths.

use serde_json::{Map, Value, json};

use executor_core::{PluginError, ToolDef, ToolName};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// One extracted HTTP operation.
#[derive(Clone, Debug)]
pub struct Operation {
    /// HTTP method (lowercase).
    pub method: String,
    /// Path template.
    pub path: String,
    /// Spec `operationId` or a synthesized stand-in.
    #[allow(clippy::struct_field_names)] // Mirrors the OpenAPI field `operationId`.
    pub operation_id: String,
    /// First tag.
    pub tag: Option<String>,
    /// Summary / description.
    pub summary: String,
    /// Parameters.
    pub parameters: Vec<Param>,
    /// JSON body schema, if any.
    pub body_schema: Option<Value>,
}

/// HTTP parameter extracted from a spec.
#[derive(Clone, Debug)]
pub struct Param {
    /// Name.
    pub name: String,
    /// `path` / `query` / `header` / `cookie`.
    pub location: String,
    /// Required?
    pub required: bool,
}

/// Parse JSON or YAML into a JSON object.
///
/// # Errors
///
/// Neither JSON nor YAML, or not an object.
pub fn parse_spec(text: &str) -> Result<Value, PluginError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(PluginError::new("empty spec"));
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && v.is_object()
    {
        return Ok(v);
    }
    let yaml: Value =
        serde_yaml::from_str(trimmed).map_err(|e| PluginError::new(format!("spec parse: {e}")))?;
    if yaml.is_object() {
        Ok(yaml)
    } else {
        Err(PluginError::new("spec must be a JSON/YAML object"))
    }
}

/// Extract operations from an `OpenAPI` 3, Swagger 2, or Google Discovery document.
///
/// # Errors
///
/// Missing paths/resources.
pub fn extract_operations(doc: &Value) -> Result<Vec<Operation>, PluginError> {
    if is_google_discovery(doc) {
        return Ok(discovery_operations(doc));
    }
    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return Err(PluginError::new("spec has no paths"));
    };
    let mut ops = Vec::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        let path_params = params_of(item.get("parameters"));
        for method in METHODS {
            let Some(op) = item.get(method).and_then(Value::as_object) else {
                continue;
            };
            ops.push(operation_from(path, method, op, &path_params, doc));
        }
    }
    Ok(ops)
}

/// Default base URL from servers / host.
#[must_use]
pub fn spec_base_url(doc: &Value) -> Option<String> {
    if let Some(url) = doc
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|s| s.get("url"))
        .and_then(Value::as_str)
    {
        return Some(url.trim_end_matches('/').to_owned());
    }
    if let Some(root) = doc.get("rootUrl").and_then(Value::as_str) {
        let base = doc.get("basePath").and_then(Value::as_str).unwrap_or("");
        return Some(format!("{}{}", root.trim_end_matches('/'), base));
    }
    let host = doc.get("host").and_then(Value::as_str)?;
    let scheme = doc
        .get("schemes")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("https");
    let base = doc.get("basePath").and_then(Value::as_str).unwrap_or("");
    Some(format!("{scheme}://{host}{}", base.trim_end_matches('/')))
}

/// Tool defs plus `plugin_meta` bindings.
///
/// # Errors
///
/// Illegal tool names (should not happen).
pub fn tools_from_operations(ops: &[Operation]) -> Result<Vec<ToolDef>, PluginError> {
    let planned = plan_tool_paths(ops);
    let mut defs = Vec::with_capacity(planned.len());
    for (path, op) in planned {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for p in &op.parameters {
            properties.insert(p.name.clone(), json!({"type":"string"}));
            if p.required {
                required.push(p.name.clone());
            }
        }
        if let Some(body) = &op.body_schema {
            if let Some(props) = body.get("properties").and_then(Value::as_object) {
                for (k, v) in props {
                    properties.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            if let Some(req) = body.get("required").and_then(Value::as_array) {
                for r in req {
                    if let Some(s) = r.as_str()
                        && !required.iter().any(|x| x == s)
                    {
                        required.push(s.to_owned());
                    }
                }
            }
            if properties.is_empty() {
                properties.insert("body".into(), body.clone());
            }
        }
        let mut schema = json!({"type":"object","properties": properties});
        if !required.is_empty() {
            schema["required"] = json!(required);
        }
        let meta = json!({
            "method": op.method,
            "path": op.path,
            "parameters": op.parameters.iter().map(|p| json!({
                "name": p.name,
                "in": p.location,
                "required": p.required,
            })).collect::<Vec<_>>(),
        });
        defs.push(ToolDef {
            name: ToolName::new(&path).map_err(PluginError::new)?,
            description: op.summary.clone(),
            input_schema: Some(schema),
            output_schema: None,
            annotations: None,
            plugin_meta: Some(meta),
        });
    }
    Ok(defs)
}

fn operation_from(
    path: &str,
    method: &str,
    op: &Map<String, Value>,
    path_params: &[Param],
    doc: &Value,
) -> Operation {
    let mut parameters = path_params.to_vec();
    parameters.extend(params_of(op.get("parameters")));
    let operation_id = op
        .get("operationId")
        .and_then(Value::as_str)
        .map_or_else(|| format!("{method}_{path}"), ToOwned::to_owned);
    let tag = op
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let summary = op
        .get("summary")
        .or_else(|| op.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Operation {
        method: method.to_owned(),
        path: path.to_owned(),
        operation_id,
        tag,
        summary,
        parameters,
        body_schema: request_body_schema(op, doc),
    }
}

fn params_of(raw: Option<&Value>) -> Vec<Param> {
    let Some(arr) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(Value::as_object)
        .filter_map(|p| {
            let location = p.get("in").and_then(Value::as_str)?;
            if !matches!(location, "path" | "query" | "header" | "cookie") {
                return None;
            }
            Some(Param {
                name: p.get("name").and_then(Value::as_str)?.to_owned(),
                location: location.to_owned(),
                required: location == "path"
                    || p.get("required").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

fn request_body_schema(op: &Map<String, Value>, doc: &Value) -> Option<Value> {
    if let Some(rb) = op.get("requestBody") {
        let resolved = resolve_ref(rb, doc);
        let content = resolved.get("content")?.as_object()?;
        let json_ct = content
            .get("application/json")
            .or_else(|| content.values().next())?;
        return json_ct.get("schema").cloned().map(|s| resolve_ref(&s, doc));
    }
    op.get("parameters")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find_map(|p| {
                let obj = p.as_object()?;
                if obj.get("in").and_then(Value::as_str) == Some("body") {
                    obj.get("schema").cloned()
                } else {
                    None
                }
            })
        })
}

fn resolve_ref(value: &Value, doc: &Value) -> Value {
    let Some(r) = value.get("$ref").and_then(Value::as_str) else {
        return value.clone();
    };
    let Some(rest) = r.strip_prefix("#/") else {
        return value.clone();
    };
    let mut cur = doc;
    for seg in rest.split('/') {
        let decoded = seg.replace("~1", "/").replace("~0", "~");
        match cur.get(&decoded) {
            Some(next) => cur = next,
            None => return value.clone(),
        }
    }
    cur.clone()
}

fn is_google_discovery(doc: &Value) -> bool {
    doc.get("kind")
        .and_then(Value::as_str)
        .is_some_and(|k| k.contains("discovery"))
        || (doc.get("resources").is_some() && doc.get("rootUrl").is_some())
}

fn discovery_operations(doc: &Value) -> Vec<Operation> {
    let mut out = Vec::new();
    walk_resources(doc.get("resources"), &mut out);
    out
}

fn walk_resources(resources: Option<&Value>, out: &mut Vec<Operation>) {
    let Some(map) = resources.and_then(Value::as_object) else {
        return;
    };
    for resource in map.values() {
        if let Some(methods) = resource.get("methods").and_then(Value::as_object) {
            for (name, method) in methods {
                if let Some(op) = discovery_method(name, method) {
                    out.push(op);
                }
            }
        }
        walk_resources(resource.get("resources"), out);
    }
}

fn discovery_method(name: &str, method: &Value) -> Option<Operation> {
    let http = method.get("httpMethod").and_then(Value::as_str)?;
    let path = method
        .get("path")
        .or_else(|| method.get("flatPath"))
        .and_then(Value::as_str)?;
    let mut parameters = Vec::new();
    if let Some(params) = method.get("parameters").and_then(Value::as_object) {
        for (pname, spec) in params {
            let location = spec
                .get("location")
                .and_then(Value::as_str)
                .unwrap_or("query");
            parameters.push(Param {
                name: pname.clone(),
                location: location.to_owned(),
                required: spec
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(location == "path"),
            });
        }
    }
    Some(Operation {
        method: http.to_ascii_lowercase(),
        path: format!("/{path}"),
        operation_id: method
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_owned(),
        tag: None,
        summary: method
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        parameters,
        body_schema: None,
    })
}

fn plan_tool_paths(ops: &[Operation]) -> Vec<(String, &Operation)> {
    let mut assigned: Vec<(String, &Operation)> = ops
        .iter()
        .map(|op| {
            let group = op
                .tag
                .as_deref()
                .map(to_camel)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| path_group(&op.path));
            let leaf = derive_leaf(&op.operation_id, &op.method, &op.path, &group);
            (format!("{group}.{leaf}"), op)
        })
        .collect();
    resolve_collisions(&mut assigned);
    assigned.sort_by(|a, b| a.0.cmp(&b.0));
    assigned
}

fn resolve_collisions(assigned: &mut [(String, &Operation)]) {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (path, _) in assigned.iter() {
        *counts.entry(path.clone()).or_insert(0) += 1;
    }
    for (path, op) in assigned.iter_mut() {
        if counts.get(path).copied().unwrap_or(0) > 1 {
            *path = format!("{path}{}", to_pascal(&op.method));
        }
    }
}

fn path_group(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .filter(|s| !s.starts_with('{'))
        .find(|s| !s.eq_ignore_ascii_case("api") && !is_version(s))
        .map_or_else(|| "root".to_owned(), to_camel)
}

fn derive_leaf(operation_id: &str, method: &str, path: &str, group: &str) -> String {
    let camel = to_camel(operation_id);
    if !camel.is_empty() && camel != group {
        return camel;
    }
    to_camel(&format!("{method}{}", to_pascal(&path_group(path))))
}

fn is_version(seg: &str) -> bool {
    let s = seg.to_ascii_lowercase();
    s.starts_with('v') && s.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
}

fn to_camel(value: &str) -> String {
    let words = split_words(value);
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i == 0 {
            out.push_str(&w.to_ascii_lowercase());
        } else {
            out.push_str(&to_pascal(w));
        }
    }
    if out.is_empty() { "tool".into() } else { out }
}

fn to_pascal(value: &str) -> String {
    let camel = to_camel(value);
    let mut c = camel.chars();
    c.next().map_or_else(String::new, |first| {
        first.to_ascii_uppercase().to_string() + c.as_str()
    })
}

fn split_words(value: &str) -> Vec<String> {
    let mut buf = String::new();
    let mut words = Vec::new();
    let chars: Vec<char> = value.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if !ch.is_ascii_alphanumeric() {
            if !buf.is_empty() {
                words.push(std::mem::take(&mut buf));
            }
            continue;
        }
        if i > 0 && ch.is_ascii_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            if (prev.is_ascii_lowercase() || (prev.is_ascii_uppercase() && next_lower))
                && !buf.is_empty()
            {
                words.push(std::mem::take(&mut buf));
            }
        }
        buf.push(*ch);
    }
    if !buf.is_empty() {
        words.push(buf);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::{extract_operations, parse_spec, tools_from_operations};

    #[test]
    fn openapi_paths_become_group_leaf() {
        let spec = r#"{
            "openapi":"3.0.0",
            "info":{"title":"Pets","version":"1"},
            "paths":{
                "/pets":{"get":{"operationId":"listPets","tags":["pets"],"responses":{"200":{}}}},
                "/pets/{id}":{"get":{"operationId":"getPet","tags":["pets"],
                    "parameters":[{"name":"id","in":"path","required":true}],
                    "responses":{"200":{}}}}
            }
        }"#;
        let doc = parse_spec(spec).unwrap();
        let ops = extract_operations(&doc).unwrap();
        let tools = tools_from_operations(&ops).unwrap();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str().to_owned()).collect();
        assert!(names.iter().any(|n| n.starts_with("pets.")), "{names:?}");
    }

    #[test]
    fn google_discovery_walks_methods() {
        let spec = r#"{
            "kind":"discovery#restDescription",
            "rootUrl":"https://gmail.googleapis.com/",
            "resources":{"users":{"methods":{"getProfile":{
                "id":"gmail.users.getProfile","httpMethod":"GET",
                "path":"gmail/v1/users/{userId}/profile",
                "parameters":{"userId":{"location":"path","required":true}}
            }}}}
        }"#;
        let doc = parse_spec(spec).unwrap();
        let ops = extract_operations(&doc).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].method, "get");
    }
}
