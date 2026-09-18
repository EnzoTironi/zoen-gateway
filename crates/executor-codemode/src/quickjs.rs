//! In-process `QuickJS` kernel — the Rust equivalent of `makeQuickJsExecutor`.
//!
//! One engine. Not Deno, not Node, not workerd. Memory / stack / interrupt
//! meters match the original local host. Tool calls pause the compute clock.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use executor_core::{ExecutionId, Limits, Outcome, PausedExecution, ToolResult};
use rquickjs::prelude::{Async, Func, Opt};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Promise, Value as JsValue,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::recover::recover_execution_body;
use super::{CodeError, CodeHost};

const GUEST_JS: &str = include_str!("guest.js");
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const MAX_STACK_BYTES: usize = 1024 * 1024;

/// How [`super::execute`] chooses a kernel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelPreference {
    /// Native subset when it parses; otherwise `QuickJS`.
    Auto,
    /// Never enter `QuickJS`.
    Native,
    /// Always `QuickJS`.
    Js,
}

impl KernelPreference {
    /// Read `EXECUTOR_KERNEL` (`auto` / `native` / `js`).
    ///
    /// # Errors
    ///
    /// Unknown value or non-Unicode env.
    pub fn from_env() -> Result<Self, CodeError> {
        match std::env::var("EXECUTOR_KERNEL") {
            Err(std::env::VarError::NotPresent) => Ok(Self::Auto),
            Err(err) => Err(CodeError::new(err.to_string())),
            Ok(raw) => parse_kernel(&raw),
        }
    }
}

fn parse_kernel(raw: &str) -> Result<KernelPreference, CodeError> {
    match raw.trim() {
        "" | "auto" => Ok(KernelPreference::Auto),
        "native" => Ok(KernelPreference::Native),
        "js" | "quickjs" => Ok(KernelPreference::Js),
        other => Err(CodeError::new(format!(
            "unknown EXECUTOR_KERNEL '{other}' (want auto|native|js)"
        ))),
    }
}

struct ToolReq {
    path: String,
    args: Value,
    reply: oneshot::Sender<String>,
}

#[derive(Clone)]
struct Deadline {
    timeout: Duration,
    start: Instant,
    in_flight: Arc<AtomicU32>,
    last_return: Arc<Mutex<Instant>>,
}

impl Deadline {
    fn new(timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            timeout,
            start: now,
            in_flight: Arc::new(AtomicU32::new(0)),
            last_return: Arc::new(Mutex::new(now)),
        }
    }

    fn expired(&self) -> bool {
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            return false;
        }
        let last = self
            .last_return
            .lock()
            .map_or_else(|e| *e.into_inner(), |g| *g);
        Instant::now().saturating_duration_since(last.max(self.start)) >= self.timeout
    }

    fn dispatch_started(&self) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
    }

    fn dispatch_returned(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut g) = self.last_return.lock() {
            *g = Instant::now();
        }
    }
}

fn isolate_timeout(limits: &Limits) -> Duration {
    let min = Duration::from_millis(100);
    if limits.execute_timeout < min {
        min
    } else {
        limits.execute_timeout
    }
}

pub async fn run(source: &str, host: &dyn CodeHost, limits: &Limits) -> Result<Outcome, CodeError> {
    let body = recover_execution_body(source);
    if body.len() > limits.max_code_bytes {
        return Err(CodeError::new(format!(
            "source is {} bytes (max {})",
            body.len(),
            limits.max_code_bytes
        )));
    }
    let timeout = isolate_timeout(limits);
    let deadline = Deadline::new(timeout);
    let paused = Arc::new(Mutex::new(None::<PausedExecution>));
    let calls = AtomicU32::new(0);
    let (tx, mut rx) = mpsc::channel::<ToolReq>(32);

    let runtime = AsyncRuntime::new().map_err(|err| CodeError::new(err.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES).await;
    runtime.set_max_stack_size(MAX_STACK_BYTES).await;
    let interrupt = deadline.clone();
    runtime
        .set_interrupt_handler(Some(Box::new(move || interrupt.expired())))
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|err| CodeError::new(err.to_string()))?;
    let script = wrap_guest(&body);

    let eval = eval_guest(context, script, tx, timeout);
    let bridge = async {
        while let Some(req) = rx.recv().await {
            deadline.dispatch_started();
            let reply = dispatch(host, limits, &calls, &paused, req.path, req.args).await;
            deadline.dispatch_returned();
            let _ = req.reply.send(reply);
        }
    };

    tokio::select! {
        result = eval => {
            if let Some(execution) = take_paused(&paused) {
                return Ok(Outcome::Paused { execution });
            }
            match result {
                Err(err) if deadline.expired() || err.0.contains("interrupt") => {
                    Err(CodeError::new(format!(
                        "QuickJS execution timed out after {}ms",
                        timeout.as_millis()
                    )))
                }
                other => other,
            }
        }
        () = bridge => Err(CodeError::new("tool bridge closed")),
    }
}

fn take_paused(paused: &Mutex<Option<PausedExecution>>) -> Option<PausedExecution> {
    paused
        .lock()
        .map_or_else(|e| e.into_inner().take(), |mut g| g.take())
}

fn wrap_guest(body: &str) -> String {
    format!("{GUEST_JS}\n(async () => {{\n{body}\n}})();\n")
}

async fn eval_guest(
    context: AsyncContext,
    script: String,
    tx: mpsc::Sender<ToolReq>,
    timeout: Duration,
) -> Result<Outcome, CodeError> {
    context
        .async_with(async move |ctx| {
            register_bridges(&ctx, tx)?;
            let promise: Promise<'_> = ctx
                .eval(script)
                .catch(&ctx)
                .map_err(|err| CodeError::new(err.to_string()))?;
            let value = promise
                .into_future::<JsValue<'_>>()
                .await
                .catch(&ctx)
                .map_err(|err| CodeError::new(err.to_string()))?;
            js_to_outcome(&ctx, value)
        })
        .await
        .map_err(|err| timeout_or(err, timeout))
}

fn timeout_or(err: CodeError, timeout: Duration) -> CodeError {
    if err.0.contains("interrupt") {
        CodeError::new(format!(
            "QuickJS execution timed out after {}ms",
            timeout.as_millis()
        ))
    } else {
        err
    }
}

fn register_bridges<'js>(ctx: &Ctx<'js>, tx: mpsc::Sender<ToolReq>) -> Result<(), CodeError> {
    let globals = ctx.globals();
    let invoke_tx = tx;
    let invoke = Function::new(
        ctx.clone(),
        Async(
            move |ctx: Ctx<'js>, path: String, args: Opt<JsValue<'js>>| {
                let payload = args.0.map_or_else(empty_object, |value| {
                    js_to_json(&ctx, value).unwrap_or(Value::Null)
                });
                let invoke_tx = invoke_tx.clone();
                async move {
                    let (reply, rx) = oneshot::channel();
                    if invoke_tx
                        .send(ToolReq {
                            path,
                            args: payload,
                            reply,
                        })
                        .await
                        .is_err()
                    {
                        return Ok(fail_json("tool bridge closed"));
                    }
                    Ok::<String, rquickjs::Error>(
                        rx.await.unwrap_or_else(|_| fail_json("tool bridge closed")),
                    )
                }
            },
        ),
    )
    .map_err(|err| CodeError::new(err.to_string()))?;
    globals
        .set("__executor_invokeTool", invoke)
        .map_err(|err| CodeError::new(err.to_string()))?;
    globals
        .set(
            "__executor_log",
            Func::from(|level: String, line: String| {
                tracing::debug!(level, line, "codemode console");
            }),
        )
        .map_err(|err| CodeError::new(err.to_string()))?;
    Ok(())
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn fail_json(error: &str) -> String {
    json!({"ok": false, "error": error}).to_string()
}

fn js_to_json<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Value, CodeError> {
    if value.is_undefined() || value.is_null() {
        return Ok(Value::Null);
    }
    let Some(s) = ctx
        .json_stringify(value)
        .map_err(|err| CodeError::new(err.to_string()))?
    else {
        return Ok(Value::Null);
    };
    let s = s
        .to_string()
        .map_err(|err| CodeError::new(err.to_string()))?;
    serde_json::from_str(&s).map_err(|err| CodeError::new(err.to_string()))
}

fn js_to_outcome<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Outcome, CodeError> {
    Ok(Outcome::Completed {
        result: ToolResult::ok(js_to_json(ctx, value)?),
        execution_id: ExecutionId::mint(),
    })
}

async fn dispatch(
    host: &dyn CodeHost,
    limits: &Limits,
    calls: &AtomicU32,
    paused: &Mutex<Option<PausedExecution>>,
    path: String,
    args: Value,
) -> String {
    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
    if n > limits.max_tool_calls {
        return fail_json(&format!(
            "max_tool_calls ({}) exceeded",
            limits.max_tool_calls
        ));
    }
    if path.is_empty() {
        return fail_json("Tool path missing in invocation");
    }
    if oversize_args(&args, limits.max_arg_bytes) {
        return fail_json(&format!(
            "tool args exceeded {} bytes",
            limits.max_arg_bytes
        ));
    }
    if path == "search" {
        return match host.search(&args).await {
            Ok(value) => ok_json(&value),
            Err(err) => fail_json(&err.to_string()),
        };
    }
    if path == "describe.tool" {
        let Some(tool_path) = args.get("path").and_then(Value::as_str) else {
            return fail_json("tools.describe.tool expects an object: { path: string }");
        };
        if args.get("includeSchemas").is_some() {
            return fail_json("tools.describe.tool no longer accepts includeSchemas");
        }
        return match host.describe_tool(tool_path).await {
            Ok(value) => ok_json(&value),
            Err(err) => fail_json(&err.to_string()),
        };
    }
    if path == "executor.integrations.list" {
        return match host.list_sandbox_integrations(&args).await {
            Ok(value) => ok_json(&value),
            Err(err) => fail_json(&err.to_string()),
        };
    }
    match host.invoke(&path, args).await {
        Ok(Outcome::Paused { execution }) => {
            if let Ok(mut g) = paused.lock() {
                *g = Some(execution);
            }
            json!({"paused": true}).to_string()
        }
        Ok(Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        }) => ok_json(&data),
        Ok(Outcome::Completed {
            result: ToolResult::Err { error },
            ..
        }) => fail_json(&error.message),
        Err(err) => fail_json(&err.to_string()),
    }
}

fn ok_json(value: &Value) -> String {
    json!({"ok": true, "value": value}).to_string()
}

fn oversize_args(args: &Value, max: usize) -> bool {
    serde_json::to_vec(args).map_or(true, |bytes| bytes.len() > max)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use executor_core::{ExecutionId, Limits, Outcome, PauseReason, PausedExecution, ToolResult};
    use serde_json::{Value, json};

    use super::{KernelPreference, parse_kernel, run};
    use crate::{CodeError, CodeHost, execute, execute_with};

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

    struct CountingHost {
        calls: AtomicU32,
    }

    #[async_trait]
    impl CodeHost for CountingHost {
        async fn invoke(&self, path: &str, args: Value) -> Result<Outcome, CodeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Outcome::Completed {
                result: ToolResult::ok(json!({"path": path, "args": args})),
                execution_id: ExecutionId::mint(),
            })
        }

        async fn search(&self, args: &Value) -> Result<Value, CodeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!([{"query": args.get("query").and_then(Value::as_str).unwrap_or("")}]))
        }
    }

    struct PauseHost;

    #[async_trait]
    impl CodeHost for PauseHost {
        async fn invoke(&self, _path: &str, _args: Value) -> Result<Outcome, CodeError> {
            Ok(Outcome::Paused {
                execution: PausedExecution {
                    id: ExecutionId::mint(),
                    reason: PauseReason::Auth {
                        message: "login".into(),
                        url: Some("https://example.test/login".into()),
                        address: None,
                        args: None,
                    },
                    expires_at_ms: 1,
                },
            })
        }

        async fn search(&self, _args: &Value) -> Result<Value, CodeError> {
            Err(CodeError::new("no search"))
        }
    }

    fn completed_data(out: Outcome) -> Value {
        match out {
            Outcome::Completed {
                result: ToolResult::Ok { data, .. },
                ..
            } => data,
            other => panic!("expected ok completion, got {other:?}"),
        }
    }

    #[test]
    fn kernel_env_values() {
        assert_eq!(parse_kernel("").unwrap(), KernelPreference::Auto);
        assert_eq!(parse_kernel("auto").unwrap(), KernelPreference::Auto);
        assert_eq!(parse_kernel("native").unwrap(), KernelPreference::Native);
        assert_eq!(parse_kernel("js").unwrap(), KernelPreference::Js);
        assert_eq!(parse_kernel("quickjs").unwrap(), KernelPreference::Js);
        assert!(parse_kernel("deno").unwrap_err().0.contains("unknown"));
    }

    #[tokio::test]
    async fn quickjs_evaluates_arithmetic() {
        let out = run("return 1 + 2;", &EchoHost, &Limits::production())
            .await
            .expect("quickjs");
        assert_eq!(completed_data(out), json!(3));
    }

    #[tokio::test]
    async fn quickjs_invokes_tools_and_search() {
        let src = r"
            const found = await tools.search({ query: 'ping' });
            const ping = await tools.echo.org.work.ping({ n: found.length });
            return { found, ping };
        ";
        let out = run(src, &EchoHost, &Limits::production())
            .await
            .expect("quickjs");
        let data = completed_data(out);
        assert_eq!(data["found"][0]["query"], "ping");
        assert_eq!(data["ping"]["path"], "echo.org.work.ping");
        assert_eq!(data["ping"]["args"]["n"], 1);
    }

    #[tokio::test]
    async fn quickjs_wraps_exported_default_arrow() {
        let out = run(
            "export default async () => 7 * 6",
            &EchoHost,
            &Limits::production(),
        )
        .await
        .expect("quickjs");
        assert_eq!(completed_data(out), json!(42));
    }

    #[tokio::test]
    async fn quickjs_surfaces_guest_throw() {
        let err = run(
            "throw new Error('guest boom');",
            &EchoHost,
            &Limits::production(),
        )
        .await
        .expect_err("throw");
        assert!(err.0.contains("guest boom"), "{err}");
    }

    #[tokio::test]
    async fn quickjs_disables_fetch() {
        let err = run(
            "return await fetch('http://127.0.0.1:1');",
            &EchoHost,
            &Limits::production(),
        )
        .await
        .expect_err("fetch");
        assert!(err.0.contains("fetch is disabled"), "{err}");
    }

    #[tokio::test]
    async fn quickjs_caps_tool_calls() {
        let host = CountingHost {
            calls: AtomicU32::new(0),
        };
        let mut limits = Limits::production();
        limits.max_tool_calls = 2;
        let src = r"
            await tools.a.b({});
            await tools.c.d({});
            await tools.e.f({});
            return 1;
        ";
        let err = run(src, &host, &limits).await.expect_err("cap");
        assert!(err.0.contains("max_tool_calls"), "{err}");
        assert_eq!(host.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn quickjs_times_out_busy_loop() {
        let mut limits = Limits::production();
        limits.execute_timeout = Duration::from_millis(200);
        let err = run("while (true) {}", &EchoHost, &limits)
            .await
            .expect_err("timeout");
        assert!(err.0.contains("timed out"), "{err}");
    }

    #[tokio::test]
    async fn quickjs_returns_paused_tool() {
        let out = run(
            r"return await tools.echo.org.work.ping({});",
            &PauseHost,
            &Limits::production(),
        )
        .await
        .expect("paused");
        assert!(matches!(out, Outcome::Paused { .. }), "{out:?}");
    }

    #[tokio::test]
    async fn auto_uses_native_for_subset_and_quickjs_for_js() {
        let native = execute(
            r#"return await tools["echo.org.work.ping"]({"n":1});"#,
            &EchoHost,
            &Limits::production(),
        )
        .await
        .expect("native");
        assert_eq!(completed_data(native)["args"]["n"], 1);

        let js = execute("return 2 + 2;", &EchoHost, &Limits::production())
            .await
            .expect("auto-js");
        assert_eq!(completed_data(js), json!(4));

        let forced = execute_with(
            "return 1 + 2;",
            &EchoHost,
            &Limits::production(),
            KernelPreference::Native,
        )
        .await
        .expect_err("native subset has no +");
        assert!(
            forced.0.contains("unexpected") || forced.0.contains("unknown"),
            "{forced}"
        );
    }
}
