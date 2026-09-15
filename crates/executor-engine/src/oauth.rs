//! `OAuth2` client-credentials, authorization-code URL, RFC 7591 DCR.
//!
//! Network calls honor `Limits.http_timeout`. Failures are structured errors,
//! not retries-without-bound.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use executor_core::{
    AuthKind, AuthMethod, CredentialMapValues, ExecutionId, ExecutorError, Outcome, PauseReason,
    PausedExecution, unix_now_ms,
};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn http() -> Client {
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .pool_max_idle_per_host(16)
                .tcp_nodelay(true)
                .build()
                .unwrap_or_else(|_| Client::new())
        })
        .clone()
}

/// Client-credentials grant. Returns the access token.
///
/// # Errors
///
/// HTTP / JSON / missing `access_token`.
pub async fn client_credentials(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    scopes: &[String],
    timeout: Duration,
) -> Result<String, ExecutorError> {
    let mut form = vec![
        ("grant_type", "client_credentials".to_owned()),
        ("client_id", client_id.to_owned()),
        ("client_secret", client_secret.to_owned()),
    ];
    if !scopes.is_empty() {
        form.push(("scope", scopes.join(" ")));
    }
    let response = http()
        .post(token_url)
        .timeout(timeout)
        .form(&form)
        .send()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("oauth token: {e}")))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("oauth token body: {e}")))?;
    if !status.is_success() {
        return Err(ExecutorError::Plugin(format!(
            "oauth token HTTP {status}: {body}"
        )));
    }
    body.get("access_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| ExecutorError::Plugin("oauth token response missing access_token".into()))
}

/// RFC 7591 dynamic client registration.
///
/// # Errors
///
/// HTTP / missing `client_id`.
pub async fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
    timeout: Duration,
) -> Result<DcrClient, ExecutorError> {
    let body = json!({
        "client_name": "executor",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let response = http()
        .post(registration_endpoint)
        .timeout(timeout)
        .json(&body)
        .send()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("dcr: {e}")))?;
    let status = response.status();
    let json: Value = response
        .json()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("dcr body: {e}")))?;
    if !status.is_success() {
        return Err(ExecutorError::Plugin(format!("dcr HTTP {status}: {json}")));
    }
    let client_id = json
        .get("client_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ExecutorError::Plugin("dcr missing client_id".into()))?
        .to_owned();
    Ok(DcrClient {
        client_id,
        client_secret: json
            .get("client_secret")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Registered OAuth client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DcrClient {
    /// Public client id.
    pub client_id: String,
    /// Optional secret (public clients omit this).
    pub client_secret: Option<String>,
}

/// Authorization-code URL with S256 PKCE.
#[must_use]
pub fn authorization_request(
    authorization_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    challenge: &str,
) -> String {
    let mut pairs = vec![
        ("response_type", "code".to_owned()),
        ("client_id", client_id.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("state", state.to_owned()),
        ("code_challenge", challenge.to_owned()),
        ("code_challenge_method", "S256".to_owned()),
    ];
    if !scopes.is_empty() {
        pairs.push(("scope", scopes.join(" ")));
    }
    if let Ok(mut parsed) = url::Url::parse(authorization_url) {
        parsed.query_pairs_mut().extend_pairs(pairs);
        parsed.to_string()
    } else {
        let qs = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        format!("{authorization_url}?{qs}")
    }
}

/// PKCE verifier + S256 challenge (no `unsafe`).
#[must_use]
pub fn pkce_pair() -> (String, String) {
    let mut raw = [0_u8; 32];
    if getrandom::getrandom(&mut raw).is_err() {
        let mixed = format!(
            "pkce-{}-{}",
            executor_core::unix_now_ms(),
            std::process::id()
        );
        raw = Sha256::digest(mixed.as_bytes()).into();
    }
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

/// Exchange an authorization code for an access token.
///
/// # Errors
///
/// HTTP / missing token.
pub async fn exchange_code(
    token_url: &str,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
    timeout: Duration,
) -> Result<String, ExecutorError> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    let response = http()
        .post(token_url)
        .timeout(timeout)
        .form(&form)
        .send()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("oauth code: {e}")))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| ExecutorError::Plugin(format!("oauth code body: {e}")))?;
    if !status.is_success() {
        return Err(ExecutorError::Plugin(format!(
            "oauth code HTTP {status}: {body}"
        )));
    }
    body.get("access_token")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| ExecutorError::Plugin("oauth code missing access_token".into()))
}

/// Fill `token` via client-credentials, or pause with an authorization-code URL.
///
/// # Errors
///
/// Token HTTP, missing OAuth inputs.
pub async fn prepare(
    methods: &[AuthMethod],
    template: &str,
    values: &mut CredentialMapValues,
    tool_path: &str,
    args: &Value,
    id: &ExecutionId,
    timeout: Duration,
) -> Result<Option<Outcome>, ExecutorError> {
    let Some(method) = methods
        .iter()
        .find(|m| m.template.as_str() == template || m.id == template)
        .cloned()
        .or_else(|| methods.iter().find(|m| m.kind == AuthKind::Oauth).cloned())
    else {
        return Ok(None);
    };
    if method.kind != AuthKind::Oauth {
        return Ok(None);
    }
    if values.get("token").is_some_and(|t| !t.is_empty()) {
        return Ok(None);
    }
    match crate::ema::try_fill_token(&method, values, timeout).await {
        Ok(true) => return Ok(None),
        Ok(false) => {}
        Err(err) if err.may_fallback() => {}
        Err(err) => return Err(ExecutorError::EnterpriseManaged(err)),
    }
    let token_url = method
        .token_url
        .clone()
        .or_else(|| values.get("token_url").cloned());
    let client_id = values.get("client_id").cloned();
    let client_secret = values.get("client_secret").cloned();
    if let (Some(url), Some(cid), Some(secret)) = (
        token_url.as_deref(),
        client_id.as_deref(),
        client_secret.as_deref(),
    ) {
        let token = client_credentials(url, cid, secret, &method.scopes, timeout).await?;
        values.insert("token".into(), token);
        return Ok(None);
    }
    let Some(auth_url) = method.authorization_url.as_deref() else {
        return Err(ExecutorError::CredentialResolution(
            "oauth connection needs token, client credentials, or authorization_url".into(),
        ));
    };
    let cid = client_id.ok_or_else(|| {
        ExecutorError::CredentialResolution("oauth authorization-code needs client_id".into())
    })?;
    let redirect = std::env::var("EXECUTOR_OAUTH_REDIRECT")
        .unwrap_or_else(|_| "http://127.0.0.1:4788/api/oauth/callback".into());
    let (verifier, challenge) = pkce_pair();
    let state = format!("{verifier}:{id}");
    let url = authorization_request(
        auth_url,
        &cid,
        &redirect,
        &method.scopes,
        &state,
        &challenge,
    );
    let ttl = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    Ok(Some(Outcome::Paused {
        execution: PausedExecution {
            id: id.clone(),
            reason: PauseReason::Auth {
                message: "Open this URL to authorize the connection, then resume.".into(),
                url: Some(url),
                address: Some(tool_path.to_owned()),
                args: Some(args.clone()),
            },
            expires_at_ms: unix_now_ms().saturating_add(ttl),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::{authorization_request, client_credentials, pkce_pair, prepare, register_client};
    use executor_core::{AuthMethod, ExecutionId, Outcome, PauseReason};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn pkce_is_s256_shaped() {
        let (verifier, challenge) = pkce_pair();
        assert!(verifier.len() >= 43, "{verifier}");
        assert!(!challenge.contains('+') && !challenge.contains('/'));
    }

    #[test]
    fn authorize_url_includes_pkce() {
        let url = authorization_request(
            "https://auth.example/authorize",
            "cid",
            "http://127.0.0.1/cb",
            &["read".into()],
            "st",
            "chal",
        );
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("scope=read"));
    }

    #[tokio::test]
    async fn client_credentials_reads_access_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"tok"})))
            .mount(&server)
            .await;
        let token = client_credentials(
            &format!("{}/token", server.uri()),
            "id",
            "secret",
            &[],
            Duration::from_secs(5),
        )
        .await
        .expect("token");
        assert_eq!(token, "tok");
    }

    #[tokio::test]
    async fn dcr_reads_client_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"client_id":"dyn"})))
            .mount(&server)
            .await;
        let client = register_client(
            &format!("{}/register", server.uri()),
            "http://127.0.0.1/cb",
            Duration::from_secs(5),
        )
        .await
        .expect("dcr");
        assert_eq!(client.client_id, "dyn");
    }

    #[tokio::test]
    async fn prepare_pauses_when_only_auth_url() {
        let methods = vec![AuthMethod::oauth(
            "https://auth.example/authorize",
            "https://auth.example/token",
            vec!["read".into()],
        )];
        let mut values = BTreeMap::from([("client_id".into(), "cid".into())]);
        let id = ExecutionId::mint();
        let paused = prepare(
            &methods,
            "oauth",
            &mut values,
            "tools.x.org.work.t",
            &json!({}),
            &id,
            Duration::from_secs(5),
        )
        .await
        .expect("prepare");
        match paused {
            Some(Outcome::Paused { execution }) => match execution.reason {
                PauseReason::Auth { url, address, .. } => {
                    let url = url.expect("auth url");
                    assert!(url.contains("code_challenge"), "{url}");
                    assert_eq!(address.as_deref(), Some("tools.x.org.work.t"));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }
}
