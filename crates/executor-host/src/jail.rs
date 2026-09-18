//! Vendor CLI jail: allowlisted basename, no shell, secret injected server-side.

use std::process::Stdio;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_catalog::cli_secret_env;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncReadExt;

use crate::AppState;

const MAX_OUTPUT: usize = 64 * 1024;

/// CLI jail HTTP group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/cli", get(list_clis))
        .route("/api/cli/run", post(run_cli))
}

#[derive(Deserialize)]
struct RunBody {
    binary: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    integration: Option<String>,
}

fn provider_for_cli(binary: &str) -> &str {
    match binary {
        "gh" => "github",
        "gcloud" => "google",
        other => other,
    }
}

async fn list_clis(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "clis": state.catalog.list_clis() }))
}

async fn run_cli(State(state): State<AppState>, Json(body): Json<RunBody>) -> impl IntoResponse {
    if !state.catalog.cli_allowed(&body.binary) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "cli_denied",
                "message": format!("{} não é um CLI permitido", body.binary),
            })),
        )
            .into_response();
    }
    let Ok(name) = body
        .binary
        .split(['/', '\\'])
        .next_back()
        .ok_or(())
        .map(str::to_owned)
    else {
        return (StatusCode::BAD_REQUEST, "binary inválido").into_response();
    };
    if !state.catalog.cli_allowed(&name) {
        return (StatusCode::FORBIDDEN, "cli não permitido").into_response();
    }
    let provider = body
        .integration
        .as_deref()
        .unwrap_or_else(|| provider_for_cli(&name));
    let secret = crate::connections::inject_secret(&state.executor, provider)
        .or_else(|| state.catalog.team_secret_for(provider));
    let tmp = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    let mut cmd = tokio::process::Command::new(&name);
    cmd.args(&body.args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", tmp.path())
        .env("TERM", "dumb")
        .current_dir(tmp.path());
    if let Some(secret) = secret.as_deref() {
        cmd.env(cli_secret_env(&name), secret);
    }
    let started = std::time::Instant::now();
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "spawn_failed",
                    "message": err.to_string(),
                    "binary": name,
                })),
            )
                .into_response();
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(30), collect_output(child)).await;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(Ok((code, mut stdout, mut stderr))) => {
            if let Some(secret) = secret.as_deref() {
                stdout = stdout.replace(secret, "***");
                stderr = stderr.replace(secret, "***");
            }
            Json(json!({
                "binary": name,
                "status": code,
                "ms": ms,
                "stdout": stdout,
                "stderr": stderr,
            }))
            .into_response()
        }
        Ok(Err(err)) => (StatusCode::BAD_GATEWAY, err).into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({ "error": "timeout", "binary": name, "ms": ms })),
        )
            .into_response(),
    }
}

async fn collect_output(mut child: tokio::process::Child) -> Result<(i32, String, String), String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout).await;
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr).await;
    }
    stdout.truncate(MAX_OUTPUT);
    stderr.truncate(MAX_OUTPUT);
    let status = child.wait().await.map_err(|e| e.to_string())?;
    Ok((
        status.code().unwrap_or(-1),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}
