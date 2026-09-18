//! Embedded console: built Next `out/` when present, otherwise the bundled SPA.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::guard::is_spa_path;

const FALLBACK_HTML: &str = include_str!("../web/index.html");

/// Serve a GET that is not an API/MCP route.
pub async fn fallback(req: Request<Body>) -> Response {
    if req.method() != axum::http::Method::GET || !is_spa_path(req.uri().path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve(req.uri().path())
}

fn serve(path: &str) -> Response {
    if let Some(file) = dist_file(path) {
        return file;
    }
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        FALLBACK_HTML,
    )
        .into_response()
}

fn dist_file(path: &str) -> Option<Response> {
    let root = console_dist()?;
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() || !trimmed.contains('.') {
        root.join("index.html")
    } else {
        root.join(trimmed)
    };
    let canonical = candidate.canonicalize().ok()?;
    if !canonical.starts_with(root.canonicalize().ok()?) {
        return None;
    }
    let bytes = std::fs::read(&canonical).ok()?;
    let ctype = content_type(canonical.extension().and_then(|e| e.to_str()).unwrap_or(""));
    Some(([(header::CONTENT_TYPE, ctype)], bytes).into_response())
}

fn console_dist() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("EXECUTOR_CONSOLE_DIST") {
        let path = PathBuf::from(dir);
        if path.join("index.html").is_file() {
            return Some(path);
        }
    }
    for rel in ["console/out", "../console/out", "out"] {
        let path = Path::new(rel);
        if path.join("index.html").is_file() {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}
