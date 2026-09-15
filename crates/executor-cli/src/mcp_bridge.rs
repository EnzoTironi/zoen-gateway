//! `executor mcp`: attach to a live daemon `/mcp` when reachable, else in-process.

use std::time::Duration;

use executor_engine::Executor;
use executor_host::{DEFAULT_PORT, stdio_loop};
use reqwest::Client;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Run the MCP stdio host.
///
/// # Errors
///
/// IO.
pub async fn run(
    executor: Executor,
    data_dir_hint: Option<&std::path::Path>,
) -> Result<(), std::io::Error> {
    let _ = data_dir_hint;
    if daemon_healthy(DEFAULT_PORT).await {
        bridge(DEFAULT_PORT).await
    } else {
        stdio_loop(executor).await
    }
}

async fn daemon_healthy(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    Client::new()
        .get(url)
        .timeout(Duration::from_millis(250))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

async fn bridge(port: u16) -> Result<(), std::io::Error> {
    let client = Client::new();
    let mcp = format!("http://127.0.0.1:{port}/mcp");
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
        let Ok(body) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let response = match client.post(&mcp).json(&body).send().await {
            Ok(resp) => resp
                .json::<Value>()
                .await
                .unwrap_or_else(|_| handle_jsonrpc_fallback(&body)),
            Err(_) => handle_jsonrpc_fallback(&body),
        };
        if response.is_null() {
            continue;
        }
        let bytes = serde_json::to_vec(&response)?;
        stdout.write_all(&bytes).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

fn handle_jsonrpc_fallback(body: &Value) -> Value {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    serde_json::json!({"jsonrpc":"2.0","id": id, "error":{"code":-32000,"message":"daemon unreachable"}})
}
