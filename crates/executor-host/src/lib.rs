//! Loopback HTTP daemon and JSON-RPC MCP host. Console UI lives in `console/`.

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::result_large_err)]

mod artifacts;
mod auth;
mod bearer;
mod billing;
mod bundles;
mod catalog_api;
mod cimd;
pub(crate) mod connections;
mod edge;
mod guard;
mod http;
mod jail;
mod mcp;
mod orgs;
mod plugins;
mod sentry;
mod skills;
mod spa;
mod well_known;

use std::net::SocketAddr;
use std::sync::Arc;

use executor_catalog::CatalogService;
use executor_core::{AtomicMetrics, Limits};
use executor_engine::Executor;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub use auth::HostAuth;
pub use bearer::{
    auth_json_path, load_or_mint as load_or_mint_auth, load_or_mint_with as load_or_mint_auth_with,
    read_token, rotate as rotate_auth,
};
pub use edge::worker_path_allowed;
pub use guard::{default_allowed_hosts, is_public};
pub use http::app;
pub use mcp::{
    ElicitationMode, McpHub, McpMode, McpOptions, SharedMcpHub, as_sse, handle_jsonrpc,
    handle_jsonrpc_with, sse_ping, stdio_loop, stdio_loop_with,
};
pub use sentry::{SentryDsn, attach as attach_sentry, attach_dsn as attach_sentry_dsn};
pub use well_known::authorization_server_metadata;

/// Bind address and resource bounds for the daemon.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Listen address. Default loopback:4788.
    pub bind: SocketAddr,
    /// Limits copied onto tower layers (body, concurrency, timeout).
    pub limits: Limits,
    /// Daemon bearer. `None` disables the gate (in-process tests).
    pub auth_token: Option<String>,
    /// CORS / Origin allow-list (`*` allowed).
    pub allowed_hosts: Vec<String>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 4788)),
            limits: Limits::production(),
            auth_token: None,
            allowed_hosts: default_allowed_hosts(),
        }
    }
}

/// Shared daemon state.
#[derive(Clone)]
pub struct AppState {
    /// Executor handle.
    pub executor: Executor,
    /// Optional scrapeable atomics.
    pub metrics: Option<Arc<AtomicMetrics>>,
    /// Device-login / OAuth tables.
    pub auth: Arc<HostAuth>,
    /// Streamable HTTP MCP sessions.
    pub mcp: Arc<McpHub>,
    /// Bearer required by [`guard`] when `Some`.
    pub auth_token: Option<String>,
    /// Allowed CORS hosts.
    pub allowed_hosts: Vec<String>,
    /// Public origin used in pause `approvalUrl`s.
    pub public_origin: String,
    /// Console origin (`/resume/{id}` chrome).
    pub console_origin: String,
    /// Treg-style catalog + ledger.
    pub catalog: Arc<CatalogService>,
}

impl AppState {
    /// Construct with an empty auth table and no daemon bearer.
    #[must_use]
    pub fn new(executor: Executor, metrics: Option<Arc<AtomicMetrics>>) -> Self {
        Self {
            executor,
            metrics,
            auth: Arc::new(HostAuth::new()),
            mcp: Arc::new(McpHub::new()),
            auth_token: None,
            allowed_hosts: default_allowed_hosts(),
            public_origin: format!("http://127.0.0.1:{DEFAULT_PORT}"),
            console_origin: default_console_origin(),
            catalog: Arc::new(CatalogService::bundled()),
        }
    }

    /// Replace the catalog (YAML ingest at daemon boot).
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<CatalogService>) -> Self {
        self.catalog = catalog;
        self
    }

    /// Attach daemon bearer, allow-list, and public origin.
    #[must_use]
    pub fn with_control(
        mut self,
        token: Option<String>,
        allowed_hosts: Vec<String>,
        public_origin: String,
    ) -> Self {
        self.auth_token = token;
        self.allowed_hosts = allowed_hosts;
        self.public_origin = public_origin;
        self
    }
}

/// Serve until `cancel` fires or the listener fails.
///
/// # Errors
///
/// Bind or server errors.
pub async fn serve(
    config: HostConfig,
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(config.bind).await?;
    let app = http::app(state, &config.limits);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel.cancelled().await;
        })
        .await
}

/// Default daemon port (matches the original).
pub const DEFAULT_PORT: u16 = 4788;

/// Durable OS service default port (matches the original).
pub const DEFAULT_SERVICE_PORT: u16 = 4789;

/// Next.js console (uncommon port; avoids 3000/5173/8080).
pub const DEFAULT_CONSOLE_PORT: u16 = 43123;

/// Console origin from `EXECUTOR_CONSOLE_ORIGIN` or the default loopback port.
#[must_use]
pub fn default_console_origin() -> String {
    std::env::var("EXECUTOR_CONSOLE_ORIGIN")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{DEFAULT_CONSOLE_PORT}"))
}
