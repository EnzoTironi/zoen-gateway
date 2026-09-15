//! Auto-start a loopback daemon so CLI verbs are HTTP clients.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use executor_host::DEFAULT_PORT;
use executor_sdk::data_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Probe `/api/health` (original CLI probe).
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
    if let Some(pointer) = read_pointer(&data)
        && is_healthy(&pointer.origin).await
    {
        return Ok(pointer.origin);
    }
    let port = free_port().unwrap_or(DEFAULT_PORT);
    spawn_daemon(&data, port)?;
    let origin = format!("http://127.0.0.1:{port}");
    for _ in 0..80 {
        if is_healthy(&origin).await {
            write_pointer(
                &data,
                &DaemonPointer {
                    origin: origin.clone(),
                    pid: 0,
                },
            )?;
            return Ok(origin);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("daemon did not become healthy at {origin}").into())
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

fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

fn spawn_daemon(data: &Path, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    fs::create_dir_all(data)?;
    let exe = std::env::current_exe()?;
    let log = fs::File::create(data.join("daemon.log"))?;
    let mut cmd = Command::new(&exe);
    cmd.args(["daemon", "run", "--port", &port.to_string(), "--foreground"])
        .env("EXECUTOR_DATA_DIR", data)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn()?;
    write_pointer(
        data,
        &DaemonPointer {
            origin: format!("http://127.0.0.1:{port}"),
            pid: child.id(),
        },
    )?;
    Ok(())
}

/// GET JSON from the daemon.
///
/// # Errors
///
/// HTTP.
pub async fn get_json(
    origin: &str,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}{path}", origin.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.json::<Value>().await.unwrap_or_else(|_| json_empty());
    if status.is_success() {
        Ok(body)
    } else {
        Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_owned()
            .into())
    }
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
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}{path}", origin.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_secs(310))
        .json(body)
        .send()
        .await?;
    let status = resp.status();
    let json = resp.json::<Value>().await.unwrap_or_else(|_| json_empty());
    if status.is_success() {
        Ok(json)
    } else {
        Err(json
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_owned()
            .into())
    }
}

fn json_empty() -> Value {
    Value::Object(serde_json::Map::new())
}
