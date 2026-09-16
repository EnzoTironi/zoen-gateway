//! Bearer + CORS + allowed-host gate for the local daemon.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// Loopback hostnames granted credentialed CORS by default.
#[must_use]
pub fn default_allowed_hosts() -> Vec<String> {
    vec![
        "127.0.0.1".into(),
        "localhost".into(),
        "::1".into(),
        "[::1]".into(),
    ]
}

/// Paths that never require the daemon bearer (health probe, CIMD, OAuth callback).
#[must_use]
pub fn is_public(path: &str) -> bool {
    path == "/health"
        || path == "/api/health"
        || path.starts_with("/.well-known/")
        || path.starts_with("/oauth/client-id-metadata")
        || path == "/api/oauth/callback"
        || path == "/api/auth/cli-login"
        || path == "/api/auth/device/code"
        || path == "/api/auth/device/token"
        || path == "/api/auth/device/verify"
}

fn request_token(headers: &HeaderMap, uri: &str) -> Option<String> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(token) = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
    {
        return Some(token.trim().to_owned());
    }
    if let Some(value) = headers
        .get("x-executor-token")
        .and_then(|v| v.to_str().ok())
    {
        return Some(value.to_owned());
    }
    uri.split('?').nth(1).and_then(|query| {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == "_token").then(|| v.to_owned())
        })
    })
}

fn origin_host(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .and_then(|origin| {
            origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"))
                .map(|rest| rest.split('/').next().unwrap_or(rest).to_owned())
        })
}

fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    allowed.iter().any(|a| {
        let a = a.split(':').next().unwrap_or(a);
        a.eq_ignore_ascii_case(host) || a == "*"
    })
}

fn with_cors(mut response: Response, headers: &HeaderMap, allowed: &[String]) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && let Some(host) = origin_host(headers)
        && host_allowed(&host, allowed)
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type, mcp-session-id"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
    }
    response
}

/// Auth + CORS middleware. No-op when the process has no daemon token (in-process tests).
pub async fn gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let allowed = &state.allowed_hosts;
    let headers = request.headers().clone();
    if request.method() == Method::OPTIONS {
        return with_cors(StatusCode::NO_CONTENT.into_response(), &headers, allowed);
    }
    let path = request.uri().path().to_owned();
    let uri = request.uri().to_string();
    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok())
        && !host_allowed(host, allowed)
        && origin_host(&headers).is_some_and(|h| !host_allowed(&h, allowed))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(expected) = &state.auth_token
        && !is_public(&path)
    {
        let presented = request_token(&headers, &uri);
        if presented.as_deref() != Some(expected.as_str()) {
            return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        }
    }
    with_cors(next.run(request).await, &headers, allowed)
}
