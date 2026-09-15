//! Loopback HTTP daemon and JSON-RPC MCP host. No UI.

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::result_large_err)]

mod http;
mod mcp;

use std::net::SocketAddr;
use std::sync::Arc;

use executor_core::{AtomicMetrics, Limits};
use executor_engine::Executor;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub use http::app;
pub use mcp::{handle_jsonrpc, stdio_loop};

/// Bind address and resource bounds for the daemon.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Listen address. Default loopback:4788.
    pub bind: SocketAddr,
    /// Limits copied onto tower layers (body, concurrency, timeout).
    pub limits: Limits,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 4788)),
            limits: Limits::production(),
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
