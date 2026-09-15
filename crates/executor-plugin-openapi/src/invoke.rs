//! HTTP invoke for an extracted spec operation.

use std::time::Duration;

use executor_core::{
    AuthKind, AuthPlacement, Carrier, PluginError, ToolError, ToolHttpMeta, ToolResult,
};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::{Map, Value, json};

const UNRESERVED: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\')
    .add(b'%');

/// Invoke using `plugin_meta` from extract.
///
/// # Errors
///
/// Transport, missing path params, illegal method.
pub async fn invoke_operation(
    client: &Client,
    base_url: &str,
    meta: &Value,
    args: &Value,
    headers: HeaderMap,
    query_extra: &[(String, String)],
    timeout: Duration,
) -> Result<ToolResult, PluginError> {
    let method = meta
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::new("missing method in plugin_meta"))?;
    let template = meta
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::new("missing path in plugin_meta"))?;
    let params = meta
        .get("parameters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let path = fill_path(template, args, &params)?;
    let url = join_url(base_url, &path);
    let method = Method::from_bytes(method.as_bytes()).map_err(PluginError::new)?;
    let mut builder = client
        .request(method, url)
        .timeout(timeout)
        .headers(headers);
    builder = apply_query(builder, args, &params, query_extra);
    builder = apply_headers(builder, args, &params)?;
    builder = apply_body(builder, args, &params);
    let response = builder
        .send()
        .await
        .map_err(|e| PluginError::new(format!("http: {e}")))?;
    let status = response.status();
    let header_pairs: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| PluginError::new(format!("body: {e}")))?;
    let data = decode_body(&bytes);
    if status.is_client_error() || status.is_server_error() {
        return Ok(ToolResult::fail(ToolError {
            code: "http_error".into(),
            message: format!("HTTP {status}"),
            status: Some(status.as_u16()),
            details: Some(data),
            retryable: Some(status.is_server_error()),
        }));
    }
    Ok(ToolResult::ok_http(
        data,
        ToolHttpMeta {
            status: status.as_u16(),
            headers: header_pairs,
        },
    ))
}

/// Render auth placements + static headers.
///
/// # Errors
///
/// Illegal header names/values.
pub fn render_headers(
    placements: &[AuthPlacement],
    values: &executor_core::CredentialMapValues,
    static_headers: &Map<String, Value>,
    kind: AuthKind,
) -> Result<HeaderMap, PluginError> {
    let mut headers = HeaderMap::new();
    if kind == AuthKind::Oauth
        && let Some(token) = values.get("token")
    {
        insert_header(&mut headers, "authorization", &format!("Bearer {token}"))?;
    }
    for p in placements {
        if p.carrier != Carrier::Header {
            continue;
        }
        let raw = p
            .literal
            .clone()
            .or_else(|| values.get(&p.variable).cloned());
        if let Some(raw) = raw {
            insert_header(&mut headers, &p.name, &format!("{}{raw}", p.prefix))?;
        }
    }
    for (k, v) in static_headers {
        if let Some(s) = v.as_str() {
            insert_header(&mut headers, k, s)?;
        }
    }
    Ok(headers)
}

/// Query placements from auth.
#[must_use]
pub fn auth_query(
    placements: &[AuthPlacement],
    values: &executor_core::CredentialMapValues,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in placements {
        if p.carrier != Carrier::Query {
            continue;
        }
        if let Some(raw) = p
            .literal
            .clone()
            .or_else(|| values.get(&p.variable).cloned())
        {
            out.push((p.name.clone(), format!("{}{raw}", p.prefix)));
        }
    }
    out
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<(), PluginError> {
    let name = HeaderName::from_bytes(name.as_bytes()).map_err(PluginError::new)?;
    let value = HeaderValue::from_str(value).map_err(PluginError::new)?;
    headers.insert(name, value);
    Ok(())
}

fn fill_path(template: &str, args: &Value, params: &[Value]) -> Result<String, PluginError> {
    let mut path = template.to_owned();
    for p in params {
        if p.get("in").and_then(Value::as_str) != Some("path") {
            continue;
        }
        let name = p.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(value) = arg_value(args, name, "path") else {
            if p.get("required").and_then(Value::as_bool).unwrap_or(false) {
                return Err(PluginError::new(format!("missing path parameter {name}")));
            }
            continue;
        };
        let encoded = utf8_percent_encode(&value_to_string(&value), UNRESERVED).to_string();
        path = path.replace(&format!("{{{name}}}"), &encoded);
    }
    Ok(path)
}

fn apply_query(
    mut builder: RequestBuilder,
    args: &Value,
    params: &[Value],
    extra: &[(String, String)],
) -> RequestBuilder {
    let mut pairs: Vec<(String, String)> = extra.to_vec();
    for p in params {
        if p.get("in").and_then(Value::as_str) != Some("query") {
            continue;
        }
        let name = p.get("name").and_then(Value::as_str).unwrap_or("");
        if let Some(value) = arg_value(args, name, "query") {
            pairs.push((name.to_owned(), value_to_string(&value)));
        }
    }
    if !pairs.is_empty() {
        builder = builder.query(&pairs);
    }
    builder
}

fn apply_headers(
    mut builder: RequestBuilder,
    args: &Value,
    params: &[Value],
) -> Result<RequestBuilder, PluginError> {
    for p in params {
        if p.get("in").and_then(Value::as_str) != Some("header") {
            continue;
        }
        let name = p.get("name").and_then(Value::as_str).unwrap_or("");
        if let Some(value) = arg_value(args, name, "header") {
            let hv = HeaderValue::from_str(&value_to_string(&value)).map_err(PluginError::new)?;
            builder = builder.header(name, hv);
        }
    }
    Ok(builder)
}

fn apply_body(builder: RequestBuilder, args: &Value, params: &[Value]) -> RequestBuilder {
    let used: Vec<&str> = params
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .collect();
    if let Some(body) = args.get("body") {
        return builder.json(body);
    }
    let Some(obj) = args.as_object() else {
        return builder;
    };
    let leftover: Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| {
            !used.contains(&k.as_str()) && *k != "query" && *k != "path" && *k != "headers"
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if leftover.is_empty() {
        builder
    } else {
        builder.json(&Value::Object(leftover))
    }
}

fn arg_value(args: &Value, name: &str, container: &str) -> Option<Value> {
    if let Some(v) = args.get(name) {
        return Some(v.clone());
    }
    args.get(container)
        .and_then(Value::as_object)
        .and_then(|m| m.get(name))
        .cloned()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_owned();
    }
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn decode_body(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(bytes) }))
}
