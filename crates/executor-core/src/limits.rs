//! Resource bounds. Exceeding one is an error, not silent growth.

use std::time::Duration;

/// Process-wide caps. Safe defaults for a multi-tenant daemon; tighten per deploy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Max concurrent `execute` / `resume` calls in this process.
    pub max_in_flight: u32,
    /// How long to wait for an in-flight slot before [`crate::ExecutorError::Overloaded`].
    /// Zero is fail-fast (no queue).
    pub acquire_timeout: Duration,
    /// Deadline for one execute, including plugin IO.
    pub execute_timeout: Duration,
    /// Deadline for a single outbound HTTP/MCP round trip.
    pub http_timeout: Duration,
    /// Max OpenAPI/GraphQL/MCP spec or introspection body.
    pub max_spec_bytes: usize,
    /// Max tools persisted for one connection.
    pub max_tools_per_connection: usize,
    /// Max tools in the whole catalog (static + dynamic).
    pub max_catalog_tools: usize,
    /// Max JSON argument payload.
    pub max_arg_bytes: usize,
    /// Max search/list page.
    pub max_search_results: usize,
    /// Pending-approval lifetime.
    pub approval_ttl: Duration,
}

impl Limits {
    /// Production defaults: bounded in-flight, 30s execute, 15s HTTP, 16 MiB specs.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_in_flight: 256,
            acquire_timeout: Duration::from_millis(0),
            execute_timeout: Duration::from_secs(30),
            http_timeout: Duration::from_secs(15),
            max_spec_bytes: 16 * 1024 * 1024,
            max_tools_per_connection: 10_000,
            max_catalog_tools: 100_000,
            max_arg_bytes: 1024 * 1024,
            max_search_results: 50,
            approval_ttl: Duration::from_mins(15),
        }
    }

    /// Tiny limits for overload/timeout tests.
    #[must_use]
    pub const fn test_tight() -> Self {
        Self {
            max_in_flight: 1,
            acquire_timeout: Duration::from_millis(0),
            execute_timeout: Duration::from_millis(50),
            http_timeout: Duration::from_millis(50),
            max_spec_bytes: 64 * 1024,
            max_tools_per_connection: 32,
            max_catalog_tools: 64,
            max_arg_bytes: 4096,
            max_search_results: 8,
            approval_ttl: Duration::from_secs(60),
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::production()
    }
}
