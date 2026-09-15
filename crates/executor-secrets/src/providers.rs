//! Extra secret backends: 1Password CLI, OS keychain, `WorkOS` Vault.
//!
//! Each call is bounded (process + HTTP timeouts). Missing binaries are
//! structured `ProviderNotRegistered` errors, not hangs.

use std::process::{Command, Stdio};
use std::time::Duration;

use executor_core::ExecutorError;

pub fn resolve(provider: &str, item: &str) -> Result<String, ExecutorError> {
    match provider {
        "1password" | "op" => op_read(item),
        "keychain" => keychain_get(item),
        "workos_vault" | "workos" => vault_get(item),
        other => Err(ExecutorError::ProviderNotRegistered(other.to_owned())),
    }
}

fn op_read(item: &str) -> Result<String, ExecutorError> {
    let output = timed_command("op", &["read", item])?;
    Ok(output.trim().to_owned())
}

fn keychain_get(item: &str) -> Result<String, ExecutorError> {
    let env_name = format!(
        "EXECUTOR_KEYCHAIN_{}",
        item.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
            .to_ascii_uppercase()
    );
    if let Ok(value) = std::env::var(&env_name) {
        return Ok(value);
    }
    timed_command("secret-tool", &["lookup", "executor", "item", item]).map_or_else(
        |_| {
            Err(ExecutorError::CredentialResolution(format!(
                "keychain item {item} not found (set {env_name} or install secret-tool)"
            )))
        },
        |value| Ok(value.trim().to_owned()),
    )
}

fn vault_get(item: &str) -> Result<String, ExecutorError> {
    let key = std::env::var("WORKOS_API_KEY").map_err(|_| {
        ExecutorError::CredentialResolution("WORKOS_API_KEY is required for workos_vault".into())
    })?;
    let base =
        std::env::var("WORKOS_VAULT_URL").unwrap_or_else(|_| "https://api.workos.com".to_owned());
    let url = format!("{}/vault/v1/kv/{item}", base.trim_end_matches('/'));
    let output = timed_command(
        "curl",
        &[
            "-fsS",
            "--max-time",
            "15",
            "-H",
            &format!("Authorization: Bearer {key}"),
            &url,
        ],
    )?;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&output)
        && let Some(value) = json
            .pointer("/value")
            .or_else(|| json.pointer("/secret"))
            .and_then(serde_json::Value::as_str)
    {
        return Ok(value.to_owned());
    }
    Ok(output.trim().to_owned())
}

fn timed_command(bin: &str, args: &[&str]) -> Result<String, ExecutorError> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ExecutorError::ProviderNotRegistered(bin.to_owned()))?;
    let timeout = Duration::from_secs(15);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return Err(ExecutorError::CredentialResolution(format!(
                    "{bin} timed out"
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(ExecutorError::CredentialResolution(e.to_string())),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| ExecutorError::CredentialResolution(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(ExecutorError::CredentialResolution(format!(
            "{bin} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}
