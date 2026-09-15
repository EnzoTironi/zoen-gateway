// Fetch proxy for the executor daemon. Allowlist must match
// `executor_host::worker_path_allowed` (crates/executor-host/src/edge.rs).
// No UI, no SQLite, no QuickJS.

function pathAllowed(pathname) {
  if (!pathname || pathname.includes("..") || pathname.includes("//")) {
    return false;
  }
  return (
    pathname === "/health" ||
    pathname === "/metrics" ||
    pathname === "/mcp" ||
    pathname.startsWith("/api/") ||
    pathname === "/.well-known/oauth-protected-resource" ||
    pathname.startsWith("/.well-known/oauth-protected-resource/") ||
    pathname === "/.well-known/oauth-authorization-server" ||
    pathname.startsWith("/.well-known/oauth-authorization-server/") ||
    pathname === "/.well-known/openid-configuration" ||
    pathname.startsWith("/.well-known/openid-configuration/")
  );
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!pathAllowed(url.pathname)) {
      return new Response("not found\n", {
        status: 404,
        headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
      });
    }
    const upstream = env.EXECUTOR_UPSTREAM;
    if (!upstream) {
      return new Response("EXECUTOR_UPSTREAM is not set\n", {
        status: 503,
        headers: { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" },
      });
    }
    const target = new URL(url.pathname + url.search, upstream);
    const headers = new Headers(request.headers);
    headers.delete("host");
    return fetch(target, {
      method: request.method,
      headers,
      body: request.body,
      redirect: "manual",
    });
  },
};
