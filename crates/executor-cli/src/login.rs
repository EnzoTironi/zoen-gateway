//! RFC 8628 device login against `GET /api/auth/cli-login`. Prints a URL; no chrome.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::{Value, json};

use crate::profiles::{self, Auth};

/// Discovery document from the server.
#[derive(Clone, Debug)]
pub struct Discovery {
    /// Device authorization URL.
    pub device_authorization_endpoint: String,
    /// Token URL.
    pub token_endpoint: String,
    /// Public client id.
    pub client_id: String,
    /// `form` or `json`.
    pub request_format: String,
    /// Optional scope.
    pub scope: Option<String>,
}

/// Discover CLI login.
///
/// # Errors
///
/// HTTP / incomplete document.
pub async fn discover(origin: &str) -> Result<Discovery, String> {
    let url = format!("{}/api/auth/cli-login", origin.trim_end_matches('/'));
    let body: Value = Client::new()
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Could not reach {origin}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("{origin} does not support CLI login (GET /api/auth/cli-login: {e})"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Discovery {
        device_authorization_endpoint: req_str(&body, "deviceAuthorizationEndpoint")?,
        token_endpoint: req_str(&body, "tokenEndpoint")?,
        client_id: req_str(&body, "clientId")?,
        request_format: body
            .get("requestFormat")
            .and_then(Value::as_str)
            .unwrap_or("form")
            .to_owned(),
        scope: body
            .get("scope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Request a device code.
///
/// # Errors
///
/// HTTP.
pub async fn request_device_code(discovery: &Discovery) -> Result<Value, String> {
    post_grant(
        discovery,
        &discovery.device_authorization_endpoint,
        json!({
            "client_id": discovery.client_id,
            "scope": discovery.scope,
        }),
    )
    .await
}

/// Poll until approved or deadline.
///
/// # Errors
///
/// Denied / timeout / HTTP.
pub async fn poll_token(
    discovery: &Discovery,
    device_code: &str,
    expires_in: u64,
    interval: u64,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(expires_in.max(1));
    let mut wait = Duration::from_secs(interval.max(1));
    loop {
        if Instant::now() >= deadline {
            return Err("Login timed out before it was approved.".into());
        }
        tokio::time::sleep(wait).await;
        let body = post_grant(
            discovery,
            &discovery.token_endpoint,
            json!({
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "device_code": device_code,
                "client_id": discovery.client_id,
            }),
        )
        .await;
        match body {
            Ok(json) if json.get("access_token").is_some() => return Ok(json),
            Ok(json) => {
                let err = json.get("error").and_then(Value::as_str).unwrap_or("");
                match err {
                    "authorization_pending" => {}
                    "slow_down" => wait += Duration::from_secs(5),
                    "access_denied" => return Err("Login was denied.".into()),
                    "expired_token" => {
                        return Err("The login request expired before it was approved.".into());
                    }
                    other => {
                        return Err(format!(
                            "Login failed: {}",
                            json.get("error_description")
                                .and_then(Value::as_str)
                                .unwrap_or(other)
                        ));
                    }
                }
            }
            Err(e) if e.contains("authorization_pending") => {}
            Err(e) => return Err(e),
        }
    }
}

async fn post_grant(discovery: &Discovery, url: &str, body: Value) -> Result<Value, String> {
    let client = Client::new();
    let req = client.post(url).timeout(Duration::from_secs(15));
    let response = if discovery.request_format == "json" {
        req.json(&body).send().await
    } else {
        let mut form = Vec::new();
        if let Some(obj) = body.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    form.push((k.clone(), s.to_owned()));
                }
            }
        }
        req.form(&form).send().await
    }
    .map_err(|e| e.to_string())?;
    let status = response.status();
    let json: Value = response.json().await.unwrap_or_else(|_| json!({}));
    if status.is_success() || json.get("error").is_some() {
        return Ok(json);
    }
    Err(format!("HTTP {status}: {json}"))
}

fn req_str(body: &Value, key: &str) -> Result<String, String> {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("CLI-login document missing {key}"))
}

/// Persist tokens onto a named profile.
///
/// # Errors
///
/// Profile IO.
pub fn store_tokens(
    data_dir: &std::path::Path,
    profile: &str,
    origin: &str,
    token_json: &Value,
) -> Result<(), String> {
    let access = token_json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "Token response was missing an access token.".to_owned())?;
    let email = token_json
        .pointer("/user/email")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let refresh = token_json
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    profiles::upsert(
        data_dir,
        profile,
        origin,
        Some(Auth::Oauth {
            access_token: access.to_owned(),
            refresh_token: refresh,
            email,
        }),
        true,
    )?;
    Ok(())
}

/// Best-effort open of an http(s) URL. Failures are ignored; the URL is printed.
pub fn try_open(url: &str) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let (bin, extra): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("rundll32.exe", &["url.dll,FileProtocolHandler"])
    } else {
        ("xdg-open", &[])
    };
    let mut cmd = std::process::Command::new(bin);
    cmd.args(extra.iter().copied());
    cmd.arg(url);
    let _ = cmd.spawn();
}
