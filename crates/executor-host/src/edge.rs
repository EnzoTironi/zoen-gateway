//! Paths the Cloudflare fetch proxy may forward. Keep in sync with `workers/proxy.js`.

/// Whether an edge Worker may reverse-proxy this path to the daemon.
///
/// Rejects UI routes, path traversal, and anything outside the HTTP/MCP API.
#[must_use]
pub fn worker_path_allowed(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path.contains("..") || path.contains("//") {
        return false;
    }
    matches!(path, "/health" | "/metrics" | "/mcp")
        || path.starts_with("/api/")
        || path == "/.well-known/oauth-protected-resource"
        || path.starts_with("/.well-known/oauth-protected-resource/")
        || path == "/.well-known/oauth-authorization-server"
        || path.starts_with("/.well-known/oauth-authorization-server/")
        || path == "/.well-known/openid-configuration"
        || path.starts_with("/.well-known/openid-configuration/")
}

#[cfg(test)]
mod tests {
    use super::worker_path_allowed;

    #[test]
    fn allows_daemon_api_only() {
        for path in [
            "/health",
            "/metrics",
            "/mcp",
            "/api/execute",
            "/api/oauth/callback",
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-authorization-server",
            "/.well-known/openid-configuration",
        ] {
            assert!(worker_path_allowed(path), "{path}");
        }
    }

    #[test]
    fn rejects_ui_and_traversal() {
        for path in [
            "/",
            "/policies",
            "/index.html",
            "/mcp/ui",
            "/api",
            "/../health",
            "/api//execute",
            "/health/../etc/passwd",
        ] {
            assert!(!worker_path_allowed(path), "{path}");
        }
    }
}
