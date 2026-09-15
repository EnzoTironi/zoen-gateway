//! Wire first-party plugins, catalog, and secrets into an [`Executor`].

#![allow(clippy::module_name_repetitions)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use executor_core::{AtomicMetrics, ExecutorError, Limits, Metrics, StorageError};
use executor_engine::Executor;
use executor_plugin_graphql::GraphqlPlugin;
use executor_plugin_mcp::McpPlugin;
use executor_plugin_openapi::OpenApiPlugin;
use executor_secrets::{FileSecrets, MemorySecrets};
use executor_storage::SqliteCatalog;

/// How to construct a process-local executor.
#[derive(Clone, Debug)]
pub struct CreateOptions {
    /// Use [`executor_core::MemoryCatalog`] (tests).
    pub in_memory: bool,
    /// Data directory. Default: `EXECUTOR_DATA_DIR` or `~/.executor`.
    pub data_dir: Option<PathBuf>,
    /// Resource bounds.
    pub limits: Limits,
    /// SQLite pool size (ignored for in-memory).
    pub pool_size: usize,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            in_memory: false,
            data_dir: None,
            limits: Limits::production(),
            pool_size: 8,
        }
    }
}

/// Resolve the catalog directory.
#[must_use]
pub fn data_dir(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var("EXECUTOR_DATA_DIR") {
        return PathBuf::from(p);
    }
    std::env::var("HOME").map_or_else(
        |_| PathBuf::from(".executor"),
        |h| PathBuf::from(h).join(".executor"),
    )
}

/// Fully wired executor: `OpenAPI` + GraphQL + MCP plugins.
///
/// # Errors
///
/// SQLite open failures.
pub fn create_executor(opts: CreateOptions) -> Result<Executor, ExecutorError> {
    create_executor_with_metrics(opts, Arc::new(executor_core::NoopMetrics))
}

/// Same as [`create_executor`] with a metrics sink.
///
/// # Errors
///
/// SQLite open failures.
pub fn create_executor_with_metrics(
    opts: CreateOptions,
    metrics: Arc<dyn Metrics>,
) -> Result<Executor, ExecutorError> {
    let mut builder = Executor::builder()
        .limits(opts.limits)
        .metrics(metrics)
        .plugin(Arc::new(OpenApiPlugin::new()))
        .plugin(Arc::new(GraphqlPlugin::new()))
        .plugin(Arc::new(McpPlugin::new()));
    if opts.in_memory {
        builder = builder.secrets(Arc::new(MemorySecrets::new()));
        return Ok(builder.build());
    }
    let dir = data_dir(opts.data_dir.as_deref());
    std::fs::create_dir_all(&dir).map_err(StorageError::new)?;
    let secrets = FileSecrets::open(dir.join("secrets.json"))?;
    let db = SqliteCatalog::open(&dir.join("catalog.db"), opts.pool_size)?;
    builder = builder.secrets(Arc::new(secrets)).store(Arc::new(db));
    Ok(builder.build())
}

/// Convenience: in-memory executor with [`AtomicMetrics`].
#[must_use]
pub fn test_executor() -> (Executor, Arc<AtomicMetrics>) {
    let metrics = Arc::new(AtomicMetrics::new());
    let exec = create_executor_with_metrics(
        CreateOptions {
            in_memory: true,
            ..CreateOptions::default()
        },
        Arc::clone(&metrics) as Arc<dyn Metrics>,
    )
    .unwrap_or_else(|_| Executor::builder().build());
    (exec, metrics)
}

#[cfg(test)]
mod tests {
    use super::{CreateOptions, create_executor};
    use executor_core::{ExecuteOptions, Outcome, ToolListFilter, ToolResult};
    use serde_json::json;

    #[tokio::test]
    async fn add_spec_lists_and_calls_mock() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/pets"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!([{"id":1}])))
            .mount(&server)
            .await;
        let spec = json!({
            "openapi":"3.0.0",
            "info":{"title":"Pets","version":"1"},
            "servers":[{"url": server.uri()}],
            "paths":{"/pets":{"get":{"operationId":"listPets","tags":["pets"],"responses":{"200":{}}}}}
        });
        let exec = create_executor(CreateOptions {
            in_memory: true,
            ..CreateOptions::default()
        })
        .expect("exec");
        let added = exec
            .execute(
                "executor.openapi.addSpec",
                json!({"slug":"pets","spec": spec.to_string(), "baseUrl": server.uri()}),
                ExecuteOptions {
                    auto_approve: true,
                    ..ExecuteOptions::default()
                },
            )
            .await
            .expect("addSpec");
        assert!(matches!(added, Outcome::Completed { .. }));
        exec.execute(
            "executor.coreTools.connections.create",
            json!({"integration":"pets","name":"work","template":"none"}),
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect("connection");
        let tools = exec
            .list_tools(&ToolListFilter {
                include_blocked: true,
                ..ToolListFilter::default()
            })
            .expect("list");
        let pet = tools
            .iter()
            .find(|t| t.integration.as_str() == "pets")
            .expect("pet tool");
        let called = exec
            .execute(
                &pet.address.to_string(),
                json!({}),
                ExecuteOptions {
                    auto_approve: true,
                    ..ExecuteOptions::default()
                },
            )
            .await
            .expect("call");
        match called {
            Outcome::Completed {
                result: ToolResult::Ok { .. },
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }
}
