//! Evaluate a parsed program against a [`super::CodeHost`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use executor_core::{ExecutionId, Limits, Outcome, PausedExecution, ToolResult};
use serde_json::{Map, Value};

use super::CodeError;
use super::CodeHost;
use super::ast::{Expr, Stmt};

enum Runtime {
    Json(Value),
    Tools(String),
    Paused(PausedExecution),
    Completed {
        result: ToolResult,
        execution_id: ExecutionId,
    },
}

enum StmtOut {
    Value(Value),
    Return(Value),
    Paused(PausedExecution),
    Completed {
        result: ToolResult,
        execution_id: ExecutionId,
    },
}

/// Run `program`.
///
/// # Errors
///
/// Unknown idents, illegal calls, host errors, step / tool-call caps.
pub async fn run(
    program: Vec<Stmt>,
    host: &dyn CodeHost,
    limits: &Limits,
) -> Result<Outcome, CodeError> {
    let mut env = BTreeMap::new();
    let mut steps = 0_u32;
    let calls = AtomicU32::new(0);
    let mut last = Value::Null;
    for stmt in program {
        match eval_stmt(stmt, &mut env, host, limits, &mut steps, &calls).await? {
            StmtOut::Value(v) => last = v,
            StmtOut::Return(v) => return Ok(completed(v)),
            StmtOut::Paused(p) => return Ok(Outcome::Paused { execution: p }),
            StmtOut::Completed {
                result,
                execution_id,
            } => {
                return Ok(Outcome::Completed {
                    result,
                    execution_id,
                });
            }
        }
    }
    Ok(completed(last))
}

async fn eval_stmt(
    stmt: Stmt,
    env: &mut BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<StmtOut, CodeError> {
    match stmt {
        Stmt::Let { name, expr } => {
            let runtime = eval_expr(expr, env, host, limits, steps, calls).await?;
            let value = expect_json(runtime)?;
            env.insert(name, value);
            Ok(StmtOut::Value(Value::Null))
        }
        Stmt::Return(expr) => match eval_expr(expr, env, host, limits, steps, calls).await? {
            Runtime::Paused(p) => Ok(StmtOut::Paused(p)),
            Runtime::Completed {
                result,
                execution_id,
            } => Ok(StmtOut::Completed {
                result,
                execution_id,
            }),
            other => Ok(StmtOut::Return(expect_json(other)?)),
        },
        Stmt::Expr(expr) => match eval_expr(expr, env, host, limits, steps, calls).await? {
            Runtime::Paused(p) => Ok(StmtOut::Paused(p)),
            Runtime::Completed { result, .. } => Ok(StmtOut::Value(result.envelope())),
            other => Ok(StmtOut::Value(expect_json(other)?)),
        },
    }
}

async fn eval_expr(
    expr: Expr,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    *steps += 1;
    if *steps > 10_000 {
        return Err(CodeError::new("eval step limit exceeded"));
    }
    match expr {
        Expr::Await(inner) => Box::pin(eval_expr(*inner, env, host, limits, steps, calls)).await,
        Expr::Literal(v) => Ok(Runtime::Json(v)),
        Expr::Ident(name) => ident_value(&name, env),
        Expr::Object(fields) => {
            Box::pin(eval_object(fields, env, host, limits, steps, calls)).await
        }
        Expr::Array(items) => Box::pin(eval_array(items, env, host, limits, steps, calls)).await,
        Expr::Member { object, prop } => {
            Box::pin(eval_member(*object, prop, env, host, limits, steps, calls)).await
        }
        Expr::Index { object, index } => {
            Box::pin(eval_index(*object, *index, env, host, limits, steps, calls)).await
        }
        Expr::Call { callee, args } => {
            Box::pin(eval_call(*callee, args, env, host, limits, steps, calls)).await
        }
    }
}

fn ident_value(name: &str, env: &BTreeMap<String, Value>) -> Result<Runtime, CodeError> {
    if name == "tools" {
        return Ok(Runtime::Tools(String::new()));
    }
    env.get(name)
        .cloned()
        .map(Runtime::Json)
        .ok_or_else(|| CodeError::new(format!("unknown identifier {name}")))
}

fn expect_json(runtime: Runtime) -> Result<Value, CodeError> {
    match runtime {
        Runtime::Json(v) => Ok(v),
        Runtime::Completed { result, .. } => Ok(result.envelope()),
        Runtime::Tools(_) => Err(CodeError::new("tools is not a JSON value")),
        Runtime::Paused(_) => Err(CodeError::new("paused execution cannot bind")),
    }
}

fn join_path(base: &str, seg: &str) -> String {
    if base.is_empty() {
        seg.to_owned()
    } else {
        format!("{base}.{seg}")
    }
}

fn completed(data: Value) -> Outcome {
    Outcome::Completed {
        result: ToolResult::ok(data),
        execution_id: ExecutionId::mint(),
    }
}

async fn eval_object(
    fields: Vec<(String, Expr)>,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    let mut map = Map::new();
    for (k, v) in fields {
        let runtime = eval_expr(v, env, host, limits, steps, calls).await?;
        map.insert(k, expect_json(runtime)?);
    }
    Ok(Runtime::Json(Value::Object(map)))
}

async fn eval_array(
    items: Vec<Expr>,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    let mut out = Vec::new();
    for item in items {
        let runtime = eval_expr(item, env, host, limits, steps, calls).await?;
        out.push(expect_json(runtime)?);
    }
    Ok(Runtime::Json(Value::Array(out)))
}

async fn eval_member(
    object: Expr,
    prop: String,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    match eval_expr(object, env, host, limits, steps, calls).await? {
        Runtime::Tools(path) => Ok(Runtime::Tools(join_path(&path, &prop))),
        Runtime::Paused(p) => Ok(Runtime::Paused(p)),
        other => json_property(expect_json(other)?, &prop),
    }
}

fn json_property(value: Value, prop: &str) -> Result<Runtime, CodeError> {
    match value {
        Value::Object(map) => map
            .get(prop)
            .cloned()
            .map(Runtime::Json)
            .ok_or_else(|| CodeError::new(format!("missing property {prop}"))),
        _ => Err(CodeError::new("property access on non-object")),
    }
}

async fn eval_index(
    object: Expr,
    index: Expr,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    let obj = eval_expr(object, env, host, limits, steps, calls).await?;
    let idx = expect_json(eval_expr(index, env, host, limits, steps, calls).await?)?;
    match (obj, idx) {
        (Runtime::Tools(_), Value::String(s)) => Ok(Runtime::Tools(s)),
        (Runtime::Paused(p), _) => Ok(Runtime::Paused(p)),
        (other, Value::String(s)) => json_property(expect_json(other)?, &s),
        _ => Err(CodeError::new("illegal index")),
    }
}

async fn eval_call(
    callee: Expr,
    args: Vec<Expr>,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Runtime, CodeError> {
    let target = eval_expr(callee, env, host, limits, steps, calls).await?;
    let Runtime::Tools(path) = target else {
        return Err(CodeError::new("can only call tools.* functions"));
    };
    let n = calls.fetch_add(1, Ordering::Relaxed) + 1;
    if n > limits.max_tool_calls {
        return Err(CodeError::new(format!(
            "max_tool_calls ({}) exceeded",
            limits.max_tool_calls
        )));
    }
    let payload = call_payload(args, env, host, limits, steps, calls).await?;
    if path == "search" {
        return Ok(Runtime::Json(host.search(&payload).await?));
    }
    if path == "describe.tool" {
        let tool_path = payload.get("path").and_then(Value::as_str).ok_or_else(|| {
            CodeError::new("tools.describe.tool expects an object: { path: string }")
        })?;
        if tool_path.is_empty() {
            return Err(CodeError::new("describe.tool requires a path"));
        }
        if payload.get("includeSchemas").is_some() {
            return Err(CodeError::new(
                "tools.describe.tool no longer accepts includeSchemas",
            ));
        }
        return Ok(Runtime::Json(host.describe_tool(tool_path).await?));
    }
    if path == "executor.integrations.list" {
        return Ok(Runtime::Json(
            host.list_sandbox_integrations(&payload).await?,
        ));
    }
    match host.invoke(&path, payload).await? {
        Outcome::Paused { execution } => Ok(Runtime::Paused(execution)),
        Outcome::Completed {
            result,
            execution_id,
        } => Ok(Runtime::Completed {
            result,
            execution_id,
        }),
    }
}

async fn call_payload(
    args: Vec<Expr>,
    env: &BTreeMap<String, Value>,
    host: &dyn CodeHost,
    limits: &Limits,
    steps: &mut u32,
    calls: &AtomicU32,
) -> Result<Value, CodeError> {
    let Some(first) = args.into_iter().next() else {
        return Ok(json_empty());
    };
    let runtime = eval_expr(first, env, host, limits, steps, calls).await?;
    expect_json(runtime)
}

fn json_empty() -> Value {
    Value::Object(Map::new())
}
