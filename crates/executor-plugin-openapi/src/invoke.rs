//! HTTP invoke for an extracted spec operation.

use std::time::Duration;

use executor_core::{
    AuthKind, AuthPlacement, Carrier, PluginError, ToolHttpMeta, ToolResult, tool_error_from_http,
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
    // OpenAPI stores methods in lowercase; HTTP/1.1 requires the token `GET`.
    let method = method.to_ascii_uppercase();
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
    // Merge auth headers. `.headers(map)` replaces the map and drops `Host`,
    // which HTTP/1.1 servers (including `npx emulate`) reject with 400.
    let mut builder = merge_headers(
        client.request(method.clone(), url).timeout(timeout),
        &headers,
    );
    builder = apply_query(builder, args, &params, query_extra);
    builder = apply_headers(builder, args, &params)?;
    builder = apply_body(builder, &method, args, &params);
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
        return Ok(ToolResult::fail(tool_error_from_http(
            status.as_u16(),
            &header_pairs,
            &data,
        )));
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

fn merge_headers(mut builder: RequestBuilder, headers: &HeaderMap) -> RequestBuilder {
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    builder
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

fn apply_body(
    builder: RequestBuilder,
    method: &Method,
    args: &Value,
    params: &[Value],
) -> RequestBuilder {
    if method == Method::GET || method == Method::HEAD {
        return builder;
    }
    let used: Vec<&str> = params
        .iter()
        .filter_map(|p| p.get("name").and_then(Value::as_str))
        .collect();
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
        return builder;
    }
    // A string `body` field is a property (GitHub issues), not the HTTP entity.
    if leftover.len() == 1
        && let Some(inner) = leftover.get("body")
        && inner.is_object()
    {
        return builder.json(inner);
    }
    builder.json(&Value::Object(leftover))
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

/// Combine a catalog `baseUrl` (often an origin override for emulate) with the spec root.
pub fn effective_base(config_base: Option<&str>, spec_base: Option<&str>) -> String {
    match (
        config_base.map(|s| s.trim_end_matches('/')),
        spec_base.map(|s| s.trim_end_matches('/')),
    ) {
        (None, None) => "http://127.0.0.1".into(),
        (Some(one), None) | (None, Some(one)) => one.to_owned(),
        (Some(config), Some(spec)) => {
            if origin_only(config) && !origin_only(spec) {
                format!("{config}{}", url_path(spec))
            } else {
                config.to_owned()
            }
        }
    }
}

fn origin_only(url: &str) -> bool {
    url_path(url).is_empty()
}

fn url_path(url: &str) -> &str {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    rest.find('/').map_or("", |i| &rest[i..])
}

fn decode_body(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(bytes) }))
}

#[cfg(test)]
mod tests {
    use super::invoke_operation;
    use executor_core::{AuthKind, AuthPlacement, CredentialMapValues, ToolResult};
    use reqwest::Client;
    use serde_json::json;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn get_keeps_http_host_and_skips_json_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 2048];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
            )
            .await
            .unwrap();
            req
        });
        let mut values = CredentialMapValues::new();
        values.insert("token".into(), "gh_test".into());
        let headers = super::render_headers(
            &[AuthPlacement::bearer_header()],
            &values,
            &serde_json::Map::new(),
            AuthKind::Header,
        )
        .unwrap();
        let result = invoke_operation(
            &Client::new(),
            &format!("http://{addr}"),
            &json!({"method": "get", "path": "/user", "parameters": []}),
            &json!({}),
            headers,
            &[],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let raw = server.await.unwrap();
        assert!(
            raw.starts_with("GET "),
            "HTTP method must be uppercase; request was:\n{raw}"
        );
        assert!(
            raw.lines()
                .any(|line| line.to_ascii_lowercase().starts_with("host:")),
            "HTTP/1.1 requires Host; request was:\n{raw}"
        );
        assert!(
            !raw.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "GET must not send a JSON body; request was:\n{raw}"
        );
        assert!(matches!(result, ToolResult::Ok { .. }), "{result:?}");
    }

    #[test]
    fn origin_override_keeps_spec_path() {
        assert_eq!(
            super::effective_base(
                Some("http://127.0.0.1:18401"),
                Some("http://localhost:18401/calendar/v3/"),
            ),
            "http://127.0.0.1:18401/calendar/v3"
        );
        assert_eq!(
            super::effective_base(
                Some("http://127.0.0.1:18400"),
                Some("http://127.0.0.1:18400"),
            ),
            "http://127.0.0.1:18400"
        );
    }

    #[tokio::test]
    async fn post_json_includes_string_body_field() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            sock.write_all(
                b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
            )
            .await
            .unwrap();
            req
        });
        let result = invoke_operation(
            &Client::new(),
            &format!("http://{addr}"),
            &json!({
                "method": "post",
                "path": "/repos/{owner}/{repo}/issues",
                "parameters": [
                    {"name": "owner", "in": "path", "required": true},
                    {"name": "repo", "in": "path", "required": true}
                ]
            }),
            &json!({
                "owner": "octocat",
                "repo": "hello-world",
                "title": "executor e2e",
                "body": "opened via the Rust port"
            }),
            reqwest::header::HeaderMap::new(),
            &[],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let raw = server.await.unwrap();
        assert!(raw.starts_with("POST "), "{raw}");
        assert!(raw.contains(r#""title":"executor e2e""#), "{raw}");
        assert!(
            raw.contains(r#""body":"opened via the Rust port""#),
            "{raw}"
        );
        assert!(matches!(result, ToolResult::Ok { .. }), "{result:?}");
    }

    #[tokio::test]
    async fn insufficient_scope_is_classified() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0_u8; 2048];
            let _ = sock.read(&mut buf).await.unwrap();
            sock.write_all(
                b"HTTP/1.1 403 Forbidden\r\nWWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"files.read\"\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
            )
            .await
            .unwrap();
        });
        let result = invoke_operation(
            &Client::new(),
            &format!("http://{addr}"),
            &json!({"method": "get", "path": "/drive", "parameters": []}),
            &json!({}),
            reqwest::header::HeaderMap::new(),
            &[],
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        server.await.unwrap();
        match result {
            ToolResult::Err { error } => {
                assert_eq!(error.code, "oauth_scope_insufficient");
            }
            ToolResult::Ok { .. } => panic!("expected oauth_scope_insufficient"),
        }
    }
}
