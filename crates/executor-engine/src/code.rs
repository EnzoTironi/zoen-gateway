//! Code-mode entry: bounded interpreter over [`crate::Executor::execute`].

use async_trait::async_trait;
use executor_codemode::{CodeError, CodeHost};
use executor_core::{ExecuteOptions, ExecutorError, Outcome, ToolListFilter};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::Executor;
use crate::execute::select_deadline;

struct EngineHost {
    exec: Executor,
    opts: ExecuteOptions,
}

#[async_trait]
impl CodeHost for EngineHost {
    async fn invoke(&self, path: &str, args: Value) -> Result<Outcome, CodeError> {
        self.exec
            .execute(path, args, self.opts.clone())
            .await
            .map_err(|e| CodeError(e.to_string()))
    }

    async fn search(&self, query: &str) -> Result<Value, CodeError> {
        let tools = self
            .exec
            .list_tools(&ToolListFilter {
                query: Some(query.to_owned()),
                ..ToolListFilter::default()
            })
            .map_err(|e| CodeError(e.to_string()))?;
        Ok(json!(
            tools
                .iter()
                .map(|t| json!({
                    "path": t.cli_path(),
                    "address": t.address.to_string(),
                    "description": t.description,
                }))
                .collect::<Vec<_>>()
        ))
    }
}

impl Executor {
    /// Run a bounded code-mode script (`return await tools[path](args)`).
    ///
    /// # Errors
    ///
    /// Oversize source, parse/eval, tool limits, or catalog execute failures.
    pub async fn run_code(
        &self,
        source: &str,
        opts: ExecuteOptions,
    ) -> Result<Outcome, ExecutorError> {
        self.run_code_with_cancel(source, opts, self.inner.cancel.child_token())
            .await
    }

    /// [`Self::run_code`] with an extra cancel token.
    ///
    /// # Errors
    ///
    /// Same as [`Self::run_code`].
    pub async fn run_code_with_cancel(
        &self,
        source: &str,
        opts: ExecuteOptions,
        cancel: CancellationToken,
    ) -> Result<Outcome, ExecutorError> {
        let timeout = opts.timeout.unwrap_or(self.inner.limits.execute_timeout);
        let host = EngineHost {
            exec: self.clone(),
            opts,
        };
        let limits = self.inner.limits.clone();
        let source = source.to_owned();
        let run = async move {
            executor_codemode::execute(&source, &host, &limits)
                .await
                .map_err(ExecutorError::from)
        };
        select_deadline(run, cancel, timeout).await
    }
}
