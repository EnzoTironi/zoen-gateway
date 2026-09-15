//! Code-mode entry: bounded interpreter over [`crate::Executor::execute`].

use async_trait::async_trait;
use executor_codemode::{CodeError, CodeHost};
use executor_core::{
    ExecuteOptions, ExecutorError, Outcome, SearchArgs, SearchableTool, json_schema_to_typescript,
    search_tools,
};
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

    async fn search(&self, args: &Value) -> Result<Value, CodeError> {
        self.exec
            .search_ranked(args)
            .map_err(|e| CodeError(e.to_string()))
    }

    async fn describe_tool(&self, path: &str) -> Result<Value, CodeError> {
        Ok(self.exec.describe_shape(path))
    }

    async fn list_sandbox_integrations(&self, args: &Value) -> Result<Value, CodeError> {
        self.exec
            .sandbox_integrations(args)
            .map_err(|e| CodeError(e.to_string()))
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
        let source = source.to_owned();
        let limits = self.inner.limits.clone();
        let run = async move {
            executor_codemode::execute(&source, &host, &limits)
                .await
                .map_err(ExecutorError::from)
        };
        select_deadline(run, cancel, timeout).await
    }

    /// Ranked `tools.search` page (`{ items, total, hasMore, nextOffset }`).
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn search_ranked(&self, args: &Value) -> Result<Value, ExecutorError> {
        let parsed = SearchArgs::from_value(args);
        let tools = self.inner.list_filtered(&executor_core::ToolListFilter {
            include_blocked: false,
            ..executor_core::ToolListFilter::default()
        })?;
        let owned: Vec<(String, String, String, String)> = tools
            .iter()
            .map(|t| {
                (
                    t.cli_path(),
                    t.name.as_str().to_owned(),
                    t.description.clone(),
                    t.integration.as_str().to_owned(),
                )
            })
            .collect();
        let view: Vec<SearchableTool<'_>> = owned
            .iter()
            .map(|(path, name, description, integration)| SearchableTool {
                path,
                name,
                description,
                integration,
            })
            .collect();
        let page = search_tools(&view, &parsed);
        serde_json::to_value(&page).map_err(|e| ExecutorError::InvalidArgs(e.to_string()))
    }

    /// Compact TypeScript shapes for `tools.describe.tool`. Missing tools are a
    /// JSON error object, not [`ExecutorError::ToolNotFound`].
    #[must_use]
    pub fn describe_shape(&self, path: &str) -> Value {
        match self.inner.resolve_path(path) {
            Ok(resolved) => {
                let tool = resolved.tool();
                let input = tool.input_schema.as_ref().map_or_else(
                    || "Record<string, unknown>".into(),
                    json_schema_to_typescript,
                );
                let output = tool
                    .output_schema
                    .as_ref()
                    .map_or_else(|| "unknown".into(), json_schema_to_typescript);
                json!({
                    "path": tool.cli_path(),
                    "name": tool.name.as_str(),
                    "description": tool.description,
                    "integration": tool.integration.as_str(),
                    "inputTypeScript": input,
                    "outputTypeScript": output,
                })
            }
            Err(err) => {
                let suggestions = match &err {
                    ExecutorError::ToolNotFound { suggestions, .. } => suggestions.clone(),
                    _ => Vec::new(),
                };
                json!({
                    "path": path,
                    "name": path,
                    "error": {
                        "code": "tool_not_found",
                        "message": format!("Tool not found: {path}"),
                        "suggestions": suggestions,
                    }
                })
            }
        }
    }

    /// Sandbox `tools.executor.integrations.list` page.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn sandbox_integrations(&self, args: &Value) -> Result<Value, ExecutorError> {
        let parsed = SearchArgs::from_value(args);
        let rows = self.list_integrations()?;
        let items: Vec<Value> = rows
            .iter()
            .map(|i| {
                json!({
                    "slug": i.slug.as_str(),
                    "name": i.name,
                    "description": i.description,
                    "kind": i.kind.as_str(),
                })
            })
            .collect();
        let page = executor_core::SearchPage::paginate(items, parsed.offset, parsed.limit);
        serde_json::to_value(&page).map_err(|e| ExecutorError::InvalidArgs(e.to_string()))
    }
}
