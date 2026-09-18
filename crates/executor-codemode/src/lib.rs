//! Bounded code-mode kernel.
//!
//! Fast path: a safe-Rust subset of `return await tools["path"](args)`.
//! Real JS: in-process `QuickJS` (`rquickjs`), the same engine executor local
//! uses via `makeQuickJsExecutor`. `EXECUTOR_KERNEL=js` forces `QuickJS`;
//! `native` disables it. There is no Deno / Node / workerd guest.
//!
//! Caps: `Limits.max_code_bytes`, `max_tool_calls`, native eval steps, `QuickJS`
//! memory (64 MiB), stack (1 MiB), and interrupt timeout (paused during tools).

#![allow(clippy::module_name_repetitions)]

mod ast;
mod eval;
mod lex;
mod parse;
mod quickjs;
mod recover;
mod strip;

use async_trait::async_trait;
use executor_core::{ExecutorError, Limits, Outcome};
use serde_json::Value;
use thiserror::Error;

pub use ast::{Expr, Stmt};
pub use eval::run;
pub use parse::parse;
pub use quickjs::KernelPreference;
pub use recover::recover_execution_body;
pub use strip::strip_typescript;

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
    /// Search visible tools (`tools.search({query, namespace, limit, offset})`).
    async fn search(&self, args: &Value) -> Result<Value, CodeError>;
    /// Compact TypeScript shapes (`tools.describe.tool({path})`).
    async fn describe_tool(&self, path: &str) -> Result<Value, CodeError> {
        Ok(serde_json::json!({
            "path": path,
            "name": path,
            "error": {
                "code": "tool_not_found",
                "message": format!("Tool not found: {path}")
            }
        }))
    }
    /// Sandbox `tools.executor.integrations.list`.
    async fn list_sandbox_integrations(&self, args: &Value) -> Result<Value, CodeError> {
        let _ = args;
        Ok(serde_json::json!({
            "items": [],
            "total": 0,
            "hasMore": false,
            "nextOffset": null
        }))
    }
}

/// Parse `source` and evaluate it against `host`.
///
/// Uses [`KernelPreference::from_env`] (`EXECUTOR_KERNEL`).
///
/// # Errors
///
/// Oversize source, parse errors, eval / tool-call limits, host failures,
/// `QuickJS` timeout / memory, or an unknown `EXECUTOR_KERNEL`.
pub async fn execute(
    source: &str,
    host: &dyn CodeHost,
    limits: &Limits,
) -> Result<Outcome, CodeError> {
    execute_with(source, host, limits, KernelPreference::from_env()?).await
}

/// [`execute`] with an explicit kernel preference.
///
/// # Errors
///
/// Same as [`execute`].
pub async fn execute_with(
    source: &str,
    host: &dyn CodeHost,
    limits: &Limits,
    preference: KernelPreference,
) -> Result<Outcome, CodeError> {
    if source.len() > limits.max_code_bytes {
        return Err(CodeError::new(format!(
            "source is {} bytes (max {})",
            source.len(),
            limits.max_code_bytes
        )));
    }
    let body = recover_execution_body(source);
    let body = strip_typescript(&body)?;
    if body.len() > limits.max_code_bytes {
        return Err(CodeError::new(format!(
            "source is {} bytes (max {})",
            body.len(),
            limits.max_code_bytes
        )));
    }
    match preference {
        KernelPreference::Js => quickjs::run(&body, host, limits).await,
        KernelPreference::Native => native(&body, host, limits).await,
        KernelPreference::Auto => match parse(&body) {
            Ok(program) => run(program, host, limits).await,
            Err(_) => quickjs::run(&body, host, limits).await,
        },
    }
}

async fn native(source: &str, host: &dyn CodeHost, limits: &Limits) -> Result<Outcome, CodeError> {
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

        async fn search(&self, args: &Value) -> Result<Value, CodeError> {
            Ok(json!([{"query": args.get("query").and_then(Value::as_str).unwrap_or("")}]))
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
    async fn strips_typescript_annotations() {
        let src = "const n: number = 2;\nreturn n + 1;";
        let out = execute(src, &EchoHost, &Limits::production())
            .await
            .expect("run");
        match out {
            Outcome::Completed {
                result: ToolResult::Ok { data, .. },
                ..
            } => assert_eq!(data, json!(3)),
            other => panic!("{other:?}"),
        }
    }
}
