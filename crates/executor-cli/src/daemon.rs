//! Auto-start a loopback daemon so CLI verbs are HTTP clients.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use executor_host::{DEFAULT_PORT, load_or_mint_auth, read_token};
use executor_sdk::data_dir;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// On-disk pointer written by [`ensure_daemon`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaemonPointer {
    /// Origin (`http://127.0.0.1:4788`).
    pub origin: String,
    /// Process id.
    pub pid: u32,
}

/// `{data_dir}/daemon.json`.
#[must_use]
pub fn pointer_path(dir: Option<&Path>) -> PathBuf {
    data_dir(dir).join("daemon.json")
}

/// Bearer from `EXECUTOR_AUTH_TOKEN` or `{data_dir}/server-control/auth.json`.
#[must_use]
pub fn bearer_token(dir: Option<&Path>) -> Option<String> {
    if let Ok(token) = std::env::var("EXECUTOR_AUTH_TOKEN")
        && !token.is_empty()
    {
        return Some(token);
    }
    read_token(&data_dir(dir))
}

/// Probe `/api/health` (original CLI probe — unauthenticated).
pub async fn is_healthy(origin: &str) -> bool {
    let url = format!("{}/api/health", origin.trim_end_matches('/'));
    reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_millis(400))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Ensure a local daemon is reachable, spawning one when the host is loopback.
///
/// Health on `:4788` is not enough: a parallel CLI with another `data_dir` can
/// own that port. The origin must accept this directory's bearer.
///
/// # Errors
///
/// Spawn failure, non-local host down, or health timeout.
pub async fn ensure_daemon(
    dir: Option<&Path>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(url) = std::env::var("EXECUTOR_DAEMON_URL")
        && !url.is_empty()
    {
        if is_healthy(&url).await {
            return Ok(url);
        }
        return Err(format!("EXECUTOR_DAEMON_URL {url} is not healthy").into());
    }
    let data = data_dir(dir);
    let token = match bearer_token(Some(&data)) {
        Some(token) => token,
        None => load_or_mint_auth(&data)?,
    };
    if let Some(pointer) = read_pointer(&data)
        && is_ours(&pointer.origin, &token).await
    {
        return Ok(pointer.origin);
    }
    let mut skip_default = false;
    let mut last_origin = String::new();
    for _ in 0..5 {
        let port = if skip_default {
            free_port().unwrap_or(DEFAULT_PORT)
        } else {
            preferred_port()
        };
        let pid = spawn_daemon(&data, port, "127.0.0.1", &[])?;
        let origin = format!("http://127.0.0.1:{port}");
        last_origin.clone_from(&origin);
        for _ in 0..80 {
            if is_ours(&origin, &token).await {
                write_pointer(
                    &data,
                    &DaemonPointer {
                        origin: origin.clone(),
                        pid,
                    },
                )?;
                return Ok(origin);
            }
            if !process_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if process_alive(pid) {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        skip_default = true;
    }
    Err(format!("daemon did not become healthy at {last_origin}").into())
}

/// `/api/health` plus a gated probe so we do not steal a neighbor's listener.
async fn is_ours(origin: &str, token: &str) -> bool {
    if !is_healthy(origin).await {
        return false;
    }
    let url = format!("{}/metrics", origin.trim_end_matches('/'));
    reqwest::Client::new()
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .timeout(Duration::from_millis(400))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

fn process_alive(pid: u32) -> bool {
    if pid <= 1 {
        return true;
    }
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Stop via pid file and pointer.
pub fn stop(dir: Option<&Path>) {
    let data = data_dir(dir);
    if let Some(pointer) = read_pointer(&data)
        && pointer.pid > 1
    {
        let _ = Command::new("kill").arg(pointer.pid.to_string()).status();
    }
    let pid_path = data.join("daemon.pid");
    if let Ok(s) = fs::read_to_string(&pid_path)
        && let Ok(pid) = s.trim().parse::<i32>()
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
        let _ = fs::remove_file(pid_path);
    }
    let _ = fs::remove_file(pointer_path(Some(&data)));
}

fn read_pointer(data: &Path) -> Option<DaemonPointer> {
    let text = fs::read_to_string(pointer_path(Some(data))).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_pointer(
    data: &Path,
    pointer: &DaemonPointer,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    fs::create_dir_all(data)?;
    fs::write(
        pointer_path(Some(data)),
        serde_json::to_vec_pretty(pointer)?,
    )?;
    Ok(())
}

fn preferred_port() -> u16 {
    if TcpListener::bind(("127.0.0.1", DEFAULT_PORT)).is_ok() {
        return DEFAULT_PORT;
    }
    free_port().unwrap_or(DEFAULT_PORT)
}

fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

/// Spawn `daemon run --foreground` detached (process group 0 on Unix).
///
/// # Errors
///
/// IO / spawn.
pub fn spawn_daemon(
    data: &Path,
    port: u16,
    hostname: &str,
    allowed_hosts: &[String],
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    fs::create_dir_all(data)?;
    let exe = std::env::current_exe()?;
    let log = fs::File::create(data.join("daemon.log"))?;
    let mut cmd = Command::new(&exe);
    cmd.args([
        "daemon",
        "run",
        "--port",
        &port.to_string(),
        "--hostname",
        hostname,
        "--foreground",
    ])
    .env("EXECUTOR_DATA_DIR", data)
    .stdin(Stdio::null())
    .stdout(log.try_clone()?)
    .stderr(log);
    for host in allowed_hosts {
        cmd.args(["--allowed-host", host]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn()?;
    let pid = child.id();
    write_pointer(
        data,
        &DaemonPointer {
            origin: format!("http://{hostname}:{port}"),
            pid,
        },
    )?;
    Ok(pid)
}

fn apply_bearer(mut req: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    if let Some(token) = token.filter(|s| !s.is_empty()) {
        req = req.header("authorization", format!("Bearer {token}"));
        req = req.header("x-treg-token", token);
    }
    req
}

fn parse_error(status: reqwest::StatusCode, text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        if let Some(m) = v.get("message").and_then(Value::as_str) {
            return m.to_owned();
        }
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            return format!("{e}: {text}");
        }
        if !text.is_empty() {
            return text.to_owned();
        }
    }
    format!("{status}: {text}")
}

async fn send_json(
    method: reqwest::Method,
    origin: &str,
    path: &str,
    body: Option<&Value>,
    token: Option<&str>,
    timeout: Duration,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}{path}", origin.trim_end_matches('/'));
    let mut req = reqwest::Client::new().request(method, url).timeout(timeout);
    if let Some(body) = body {
        req = req.json(body);
    }
    let resp = apply_bearer(req, token).send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status.is_success() || status.as_u16() == 204 {
        if text.is_empty() {
            return Ok(json_empty());
        }
        return Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({"ok": true, "raw": text})));
    }
    Err(parse_error(status, &text).into())
}

/// GET JSON from the daemon.
///
/// # Errors
///
/// HTTP.
pub async fn get_json(
    origin: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    send_json(
        reqwest::Method::GET,
        origin,
        path,
        None,
        token,
        Duration::from_secs(15),
    )
    .await
}

/// POST JSON to the daemon.
///
/// # Errors
///
/// HTTP.
pub async fn post_json(
    origin: &str,
    path: &str,
    body: &Value,
    token: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    send_json(
        reqwest::Method::POST,
        origin,
        path,
        Some(body),
        token,
        Duration::from_secs(310),
    )
    .await
}

/// DELETE a daemon path.
///
/// # Errors
///
/// HTTP.
pub async fn delete_json(
    origin: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    send_json(
        reqwest::Method::DELETE,
        origin,
        path,
        None,
        token,
        Duration::from_secs(15),
    )
    .await
}

fn json_empty() -> Value {
    Value::Object(serde_json::Map::new())
}
