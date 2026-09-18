//! `executor mcp`: always a Streamable HTTP client of the local daemon.

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Bridge stdio JSON-RPC to `{origin}/mcp{query}`.
///
/// # Errors
///
/// IO.
pub async fn run(origin: &str, query: &str, token: Option<&str>) -> Result<(), std::io::Error> {
    let client = Client::new();
    let mcp = format!("{}/mcp{}", origin.trim_end_matches('/'), query);
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    let mut session: Option<String> = None;
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
        let mut req = client
            .post(&mcp)
            .timeout(Duration::from_secs(310))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(token) = token.filter(|s| !s.is_empty()) {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        if let Some(id) = &session {
            req = req.header("mcp-session-id", id);
        }
        let response = match req.send().await {
            Ok(resp) => {
                if let Some(id) = resp.headers().get("mcp-session-id")
                    && let Ok(s) = id.to_str()
                {
                    session = Some(s.to_owned());
                }
                resp.json::<Value>()
                    .await
                    .unwrap_or_else(|_| fallback(&body, "daemon returned non-JSON"))
            }
            Err(err) => fallback(&body, &err.to_string()),
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

fn fallback(body: &Value, message: &str) -> Value {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    serde_json::json!({"jsonrpc":"2.0","id": id, "error":{"code":-32000,"message": message}})
}
