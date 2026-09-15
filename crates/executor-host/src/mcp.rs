//! JSON-RPC MCP: `initialize`, `tools/list`, `tools/call`, `ping`.

use executor_core::{ExecuteOptions, ToolListFilter};
use executor_engine::Executor;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::instrument;

/// Handle a single JSON-RPC object (or a notification).
pub async fn handle_jsonrpc(executor: &Executor, body: Value) -> Value {
    if let Some(arr) = body.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(handle_one(executor, item.clone()).await);
        }
        return Value::Array(out);
    }
    handle_one(executor, body).await
}

#[instrument(skip(executor, body))]
async fn handle_one(executor: &Executor, body: Value) -> Value {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));
    if body.get("id").is_none() {
        return Value::Null;
    }
    match method {
        "initialize" => ok(
            &id,
            &json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "executor", "version": "0.1.0"},
            }),
        ),
        "ping" => ok(&id, &json!({})),
        "tools/list" => match executor.list_tools(&ToolListFilter::default()) {
            Ok(tools) => ok(
                &id,
                &json!({
                    "tools": tools.iter().map(|t| json!({
                        "name": t.cli_path(),
                        "description": t.description,
                        "inputSchema": t.input_schema.clone().unwrap_or_else(|| json!({"type":"object"})),
                    })).collect::<Vec<_>>()
                }),
            ),
            Err(e) => rpc_error(&id, -32000, e.to_string()),
        },
        "tools/call" => call_tool(executor, id, params).await,
        other => rpc_error(&id, -32601, format!("method not found: {other}")),
    }
}

async fn call_tool(executor: &Executor, id: Value, params: Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(&id, -32602, "missing name");
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match executor
        .execute(
            name,
            args,
            ExecuteOptions {
                auto_approve: false,
                timeout: None,
                idempotency_key: None,
            },
        )
        .await
    {
        Ok(outcome) => ok(
            &id,
            &json!({
                "content": [{"type":"text","text": outcome.cli_json().to_string()}],
                "structuredContent": outcome.cli_json(),
            }),
        ),
        Err(e) => rpc_error(&id, jsonrpc_code(&e), e.to_string()),
    }
}

const fn jsonrpc_code(err: &executor_core::ExecutorError) -> i64 {
    match err {
        executor_core::ExecutorError::Overloaded { .. } => -32000,
        executor_core::ExecutorError::Timeout { .. } => -32001,
        executor_core::ExecutorError::Cancelled => -32002,
        executor_core::ExecutorError::ToolNotFound { .. } => -32601,
        executor_core::ExecutorError::InvalidArgs(_) => -32602,
        executor_core::ExecutorError::EnterpriseManaged(
            executor_core::EmaError::SubjectTokenRejected { .. },
        ) => -32004,
        _ => -32003,
    }
}

fn ok(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc":"2.0","id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id": id, "error": {"code": code, "message": message.into()}})
}

/// Stdio MCP: newline-delimited JSON (and Content-Length frames).
///
/// # Errors
///
/// IO on stdin/stdout.
pub async fn stdio_loop(executor: Executor) -> Result<(), std::io::Error> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Content-Length:") {
            drain_content_length(&mut reader, &mut stdout, &executor, trimmed).await?;
            continue;
        }
        let Ok(body) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let response = handle_jsonrpc(&executor, body).await;
        if !response.is_null() {
            let bytes = serde_json::to_vec(&response)?;
            stdout.write_all(&bytes).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

async fn drain_content_length(
    reader: &mut BufReader<tokio::io::Stdin>,
    stdout: &mut tokio::io::Stdout,
    executor: &Executor,
    first: &str,
) -> Result<(), std::io::Error> {
    let mut headers = first.to_owned();
    headers.push('\n');
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
        headers.push_str(&line);
    }
    let len = headers.lines().find_map(|l| {
        l.strip_prefix("Content-Length:")
            .and_then(|s| s.trim().parse::<usize>().ok())
    });
    let Some(len) = len else {
        return Ok(());
    };
    let mut buf = vec![0u8; len];
    tokio::io::AsyncReadExt::read_exact(reader, &mut buf).await?;
    let Ok(body) = serde_json::from_slice::<Value>(&buf) else {
        return Ok(());
    };
    let response = handle_jsonrpc(executor, body).await;
    if response.is_null() {
        return Ok(());
    }
    let payload = serde_json::to_vec(&response)?;
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    stdout.write_all(header.as_bytes()).await?;
    stdout.write_all(&payload).await?;
    stdout.flush().await?;
    Ok(())
}
