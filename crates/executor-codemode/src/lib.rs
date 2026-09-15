//! Bounded code-mode kernel. Not `QuickJS`: a safe-Rust subset of
//! `return await tools["path"](args)` / `tools.search({query})`.
//!
//! Caps (`Limits.max_code_bytes`, `max_tool_calls`, eval steps) keep a noisy
//! script from unbounded work. Each tool call still goes through `execute`.

#![allow(clippy::module_name_repetitions)]

mod ast;
mod eval;
mod lex;
mod parse;

use async_trait::async_trait;
use executor_core::{ExecutorError, Limits, Outcome};
use serde_json::Value;
use thiserror::Error;

pub use ast::{Expr, Stmt};
pub use eval::run;
pub use parse::parse;

/// Compile a single catalog call into the subset grammar.
#[must_use]
pub fn compile_call(path: &str, args: &Value) -> String {
    executor_core::compile_call(path, args)
}

/// Code-mode parse / runtime failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0}")]
pub struct CodeError(pub String);

impl CodeError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<CodeError> for ExecutorError {
    fn from(value: CodeError) -> Self {
        Self::Code(value.0)
    }
}

/// Catalog side of the kernel. Engine implements this; tests stub it.
#[async_trait]
pub trait CodeHost: Send + Sync {
    /// Invoke one catalog / static tool.
    async fn invoke(&self, path: &str, args: Value) -> Result<Outcome, CodeError>;
    /// Search visible tools (`tools.search({query})`).
    async fn search(&self, query: &str) -> Result<Value, CodeError>;
}

/// Parse `source` and evaluate it against `host`.
///
/// # Errors
///
/// Oversize source, parse errors, eval / tool-call limits, host failures.
pub async fn execute(
    source: &str,
    host: &dyn CodeHost,
    limits: &Limits,
) -> Result<Outcome, CodeError> {
    if source.len() > limits.max_code_bytes {
        return Err(CodeError::new(format!(
            "source is {} bytes (max {})",
            source.len(),
            limits.max_code_bytes
        )));
    }
    let program = parse(source)?;
    run(program, host, limits).await
}

#[cfg(test)]
mod tests {
    use super::{CodeError, CodeHost, compile_call, execute};
    use async_trait::async_trait;
    use executor_core::{ExecutionId, Limits, Outcome, ToolResult};
    use serde_json::{Value, json};

    struct EchoHost;

    #[async_trait]
    impl CodeHost for EchoHost {
        async fn invoke(&self, path: &str, args: Value) -> Result<Outcome, CodeError> {
            Ok(Outcome::Completed {
                result: ToolResult::ok(json!({"path": path, "args": args})),
                execution_id: ExecutionId::mint(),
            })
        }

        async fn search(&self, query: &str) -> Result<Value, CodeError> {
            Ok(json!([{"query": query}]))
        }
    }

    #[tokio::test]
    async fn compiled_call_invokes_host() {
        let src = compile_call("echo.org.work.ping", &json!({"n": 1}));
        let out = execute(&src, &EchoHost, &Limits::production())
            .await
            .expect("run");
        match out {
            Outcome::Completed {
                result: ToolResult::Ok { data, .. },
                ..
            } => {
                assert_eq!(data["path"], "echo.org.work.ping");
                assert_eq!(data["args"]["n"], 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn search_builtin() {
        let src = r#"return await tools.search({"query":"issue"});"#;
        let out = execute(src, &EchoHost, &Limits::production())
            .await
            .expect("run");
        match out {
            Outcome::Completed {
                result: ToolResult::Ok { data, .. },
                ..
            } => assert_eq!(data[0]["query"], "issue"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_oversize_source() {
        let src = "return 1;".repeat(200);
        let err = execute(&src, &EchoHost, &Limits::test_tight())
            .await
            .expect_err("limit");
        assert!(err.0.contains("bytes"), "{err}");
    }

    #[tokio::test]
    async fn rejects_unknown_ident() {
        let err = execute("return foo;", &EchoHost, &Limits::production())
            .await
            .expect_err("ident");
        assert!(err.0.contains("unknown"), "{err}");
    }
}
