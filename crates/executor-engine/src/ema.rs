//! MCP Enterprise-Managed Authorization: RFC 8693 exchange + RFC 7523 redeem.
//!
//! The ID-JAG itself is never retained. Renewal re-runs the chain (draft §4.4.3).

use std::time::Duration;

use executor_core::{
    AuthMethod, CredentialMapValues, DEFAULT_SUBJECT_TOKEN_TYPE, EmaError, EmaStep,
    ID_JAG_TOKEN_TYPE, ID_JAG_TOKEN_TYPE_SENTINEL, JWT_BEARER_GRANT_TYPE,
    TOKEN_EXCHANGE_GRANT_TYPE, supports_id_jag_grant_profile,
};
use serde_json::{Value, json};

use crate::oauth::http;

/// An access token minted through the ID-JAG chain. No refresh token is kept.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnterpriseManagedGrant {
    /// Resource-AS access token.
    pub access_token: String,
    /// Granted scope as echoed by the Resource AS, else the `IdP`, else empty.
    pub scope: String,
}

/// Inputs for [`mint_enterprise_managed_access_token`].
#[derive(Clone, Debug)]
pub struct EmaMintInput<'a> {
    /// `IdP` token endpoint (RFC 8693).
    pub idp_token_url: &'a str,
    /// Client id at the `IdP` (draft §5: different from the resource-AS client).
    pub idp_client_id: &'a str,
    /// Optional `IdP` client secret.
    pub idp_client_secret: Option<&'a str>,
    /// Identity assertion from enterprise SSO.
    pub subject_token: &'a str,
    /// RFC 8693 `subject_token_type`.
    pub subject_token_type: &'a str,
    /// Resource AS token endpoint.
    pub resource_token_url: &'a str,
    /// Resource AS issuer (`aud` of the ID-JAG).
    pub audience: &'a str,
    /// Resource AS client id.
    pub resource_client_id: &'a str,
    /// Optional resource-AS client secret.
    pub resource_client_secret: Option<&'a str>,
    /// Optional RFC 8707 resource indicator.
    pub resource: Option<&'a str>,
    /// Requested scopes.
    pub scopes: &'a [String],
}

/// RFC 8414 Authorization Server metadata (the subset EMA reads).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationServerMetadata {
    /// Issuer identifier.
    pub issuer: String,
    /// Token endpoint.
    pub token_endpoint: String,
    /// Advertised grant profiles. Empty when the field is absent.
    pub authorization_grant_profiles_supported: Vec<String>,
}

/// Discover RFC 8414 / OIDC metadata. `None` when nothing is advertised.
///
/// # Errors
///
/// Transport failures after a non-404 response that is not metadata.
pub async fn discover_authorization_server_metadata(
    issuer: &str,
    timeout: Duration,
) -> Result<Option<AuthorizationServerMetadata>, EmaError> {
    let trimmed = issuer.trim_end_matches('/');
    let Ok(url) = url::Url::parse(trimmed) else {
        return Ok(None);
    };
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let has_path = !path.is_empty() && path != "/";
    let mut candidates = vec![
        insert_well_known(
            &origin,
            "oauth-authorization-server",
            has_path.then_some(path),
        ),
        insert_well_known(&origin, "openid-configuration", has_path.then_some(path)),
    ];
    if has_path {
        candidates.push(format!("{origin}{path}/.well-known/openid-configuration"));
    }
    for metadata_url in candidates {
        match fetch_metadata(&metadata_url, timeout).await {
            Ok(Some(meta)) => return Ok(Some(meta)),
            Ok(None) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

/// RFC 9728 well-known URLs for a protected resource (path-scoped then origin).
#[must_use]
pub fn resource_metadata_urls(resource_url: &str) -> Vec<String> {
    let Ok(url) = url::Url::parse(resource_url) else {
        return Vec::new();
    };
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() && path != "/" {
        urls.push(format!(
            "{origin}/.well-known/oauth-protected-resource{path}"
        ));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    urls
}

/// Detect the ID-JAG profile, then mint. Missing/unadvertised profile is fallback-safe.
///
/// # Errors
///
/// Any [`EmaError`].
pub async fn run_enterprise_managed_authorization(
    input: EmaMintInput<'_>,
    metadata: &AuthorizationServerMetadata,
    timeout: Duration,
) -> Result<EnterpriseManagedGrant, EmaError> {
    if !supports_id_jag_grant_profile(&metadata.authorization_grant_profiles_supported) {
        return Err(EmaError::GrantProfileUnsupported {
            issuer: metadata.issuer.clone(),
            advertised: metadata.authorization_grant_profiles_supported.clone(),
        });
    }
    let rebound = EmaMintInput {
        resource_token_url: metadata.token_endpoint.as_str(),
        audience: metadata.issuer.as_str(),
        ..input
    };
    mint_enterprise_managed_access_token(&rebound, timeout).await
}

/// Two-step grant. Does not inspect metadata (so it cannot yield `GrantProfileUnsupported`).
///
/// # Errors
///
/// Policy, subject-token, redemption, or upstream failures.
pub async fn mint_enterprise_managed_access_token(
    input: &EmaMintInput<'_>,
    timeout: Duration,
) -> Result<EnterpriseManagedGrant, EmaError> {
    let grant = exchange_subject_token_for_id_jag(input, timeout).await?;
    let granted: Vec<String> = grant.scope.as_deref().map_or_else(
        || input.scopes.to_vec(),
        |s| s.split_whitespace().map(ToOwned::to_owned).collect(),
    );
    let token = redeem_id_jag_assertion(input, &grant.assertion, &granted, timeout).await?;
    Ok(EnterpriseManagedGrant {
        access_token: token.access_token,
        scope: token.scope.or(grant.scope).unwrap_or_default(),
    })
}

/// Fill `token` via EMA when the connection is fully wired. `Ok(false)` means skip.
///
/// # Errors
///
/// EMA failures that must not fall back (or `GrantProfileUnsupported`).
pub async fn try_fill_token(
    method: &AuthMethod,
    values: &mut CredentialMapValues,
    timeout: Duration,
) -> Result<bool, EmaError> {
    if !ema_rollout_enabled() {
        return Ok(false);
    }
    let Some(input) = mint_input_from(method, values) else {
        return Ok(false);
    };
    let audience = input.audience.to_owned();
    let metadata = discover_authorization_server_metadata(&audience, timeout).await?;
    let grant = match metadata.as_ref() {
        Some(meta) => run_enterprise_managed_authorization(input, meta, timeout).await?,
        None => {
            return Err(EmaError::GrantProfileUnsupported {
                issuer: audience,
                advertised: Vec::new(),
            });
        }
    };
    values.insert("token".into(), grant.access_token);
    if !grant.scope.is_empty() {
        values.insert("scope".into(), grant.scope);
    }
    Ok(true)
}

fn ema_rollout_enabled() -> bool {
    std::env::var("EXECUTOR_EMA").map_or(true, |value| {
        let lower = value.to_ascii_lowercase();
        !matches!(lower.as_str(), "0" | "false" | "off" | "disabled")
    })
}

struct IdJagGrant {
    assertion: String,
    scope: Option<String>,
}

struct TokenGrant {
    access_token: String,
    scope: Option<String>,
}

fn mint_input_from<'a>(
    method: &'a AuthMethod,
    values: &'a CredentialMapValues,
) -> Option<EmaMintInput<'a>> {
    let subject_token = nonempty(values.get("identity_assertion")?)?;
    let idp_token_url = nonempty(values.get("idp_token_url")?)?;
    let idp_client_id = nonempty(values.get("idp_client_id")?)?;
    let audience = values
        .get("audience")
        .or_else(|| values.get("issuer"))
        .and_then(|s| nonempty(s))?;
    let resource_client_id = nonempty(values.get("client_id")?)?;
    let resource_token_url = method
        .token_url
        .as_deref()
        .or_else(|| values.get("token_url").map(String::as_str))
        .and_then(|s| {
            let t = s.trim();
            (!t.is_empty()).then_some(t)
        })?;
    Some(EmaMintInput {
        idp_token_url,
        idp_client_id,
        idp_client_secret: values.get("idp_client_secret").map(String::as_str),
        subject_token,
        subject_token_type: values
            .get("subject_token_type")
            .map_or(DEFAULT_SUBJECT_TOKEN_TYPE, String::as_str),
        resource_token_url,
        audience,
        resource_client_id,
        resource_client_secret: values.get("client_secret").map(String::as_str),
        resource: values.get("resource").map(String::as_str),
        scopes: method.scopes.as_slice(),
    })
}

fn nonempty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

async fn exchange_subject_token_for_id_jag(
    input: &EmaMintInput<'_>,
    timeout: Duration,
) -> Result<IdJagGrant, EmaError> {
    let mut form = vec![
        ("grant_type", TOKEN_EXCHANGE_GRANT_TYPE.to_owned()),
        ("requested_token_type", ID_JAG_TOKEN_TYPE.to_owned()),
        ("audience", input.audience.to_owned()),
        ("subject_token", input.subject_token.to_owned()),
        ("subject_token_type", input.subject_token_type.to_owned()),
        ("client_id", input.idp_client_id.to_owned()),
    ];
    if let Some(secret) = input.idp_client_secret {
        form.push(("client_secret", secret.to_owned()));
    }
    if let Some(resource) = input.resource {
        form.push(("resource", resource.to_owned()));
    }
    if !input.scopes.is_empty() {
        form.push(("scope", input.scopes.join(" ")));
    }
    let (status, body) =
        post_form(input.idp_token_url, &form, timeout, EmaStep::TokenExchange).await?;
    if !status {
        return Err(exchange_failure(&body));
    }
    parse_id_jag_body(&body)
}

async fn redeem_id_jag_assertion(
    input: &EmaMintInput<'_>,
    assertion: &str,
    scopes: &[String],
    timeout: Duration,
) -> Result<TokenGrant, EmaError> {
    let mut form = vec![
        ("grant_type", JWT_BEARER_GRANT_TYPE.to_owned()),
        ("assertion", assertion.to_owned()),
        ("client_id", input.resource_client_id.to_owned()),
    ];
    if let Some(secret) = input.resource_client_secret {
        form.push(("client_secret", secret.to_owned()));
    }
    if let Some(resource) = input.resource {
        form.push(("resource", resource.to_owned()));
    }
    if !scopes.is_empty() {
        form.push(("scope", scopes.join(" ")));
    }
    let (status, body) = post_form(
        input.resource_token_url,
        &form,
        timeout,
        EmaStep::Redemption,
    )
    .await?;
    if !status {
        return Err(redemption_failure(&body));
    }
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| EmaError::UpstreamUnavailable {
            step: EmaStep::Redemption,
            detail: "token response missing access_token".into(),
        })?;
    Ok(TokenGrant {
        access_token: access_token.to_owned(),
        scope: body
            .get("scope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

async fn post_form(
    url: &str,
    form: &[(&str, String)],
    timeout: Duration,
    step: EmaStep,
) -> Result<(bool, Value), EmaError> {
    let response = http()
        .post(url)
        .timeout(timeout)
        .form(form)
        .send()
        .await
        .map_err(|e| EmaError::UpstreamUnavailable {
            step,
            detail: e.to_string(),
        })?;
    let ok = response.status().is_success();
    let body = response.json().await.unwrap_or_else(|_| json!({}));
    Ok((ok, body))
}

fn parse_id_jag_body(body: &Value) -> Result<IdJagGrant, EmaError> {
    let assertion = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| EmaError::UpstreamUnavailable {
            step: EmaStep::TokenExchange,
            detail: "ID-JAG token exchange response did not match RFC 8693 §2.2.1".into(),
        })?;
    let issued = body.get("issued_token_type").and_then(Value::as_str);
    if issued != Some(ID_JAG_TOKEN_TYPE) {
        return Err(EmaError::UpstreamUnavailable {
            step: EmaStep::TokenExchange,
            detail: format!(
                "ID-JAG token exchange returned issued_token_type {:?}, expected \"{ID_JAG_TOKEN_TYPE}\"",
                issued.unwrap_or("")
            ),
        });
    }
    let token_type = body.get("token_type").and_then(Value::as_str);
    if token_type != Some(ID_JAG_TOKEN_TYPE_SENTINEL) {
        return Err(EmaError::UpstreamUnavailable {
            step: EmaStep::TokenExchange,
            detail: format!(
                "ID-JAG token exchange returned token_type {:?}, expected \"{ID_JAG_TOKEN_TYPE_SENTINEL}\"",
                token_type.unwrap_or("")
            ),
        });
    }
    Ok(IdJagGrant {
        assertion: assertion.to_owned(),
        scope: body
            .get("scope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn exchange_failure(body: &Value) -> EmaError {
    let (code, detail) = oauth_error_from_body(body, "ID-JAG token exchange was rejected");
    match code.as_deref() {
        Some("invalid_grant") => EmaError::SubjectTokenRejected { detail },
        Some(error) => EmaError::PolicyDenied {
            error: error.to_owned(),
            detail,
        },
        None => EmaError::UpstreamUnavailable {
            step: EmaStep::TokenExchange,
            detail,
        },
    }
}

fn redemption_failure(body: &Value) -> EmaError {
    let (code, detail) = oauth_error_from_body(body, "ID-JAG redemption was rejected");
    match code {
        Some(error) => EmaError::RedemptionRejected {
            error: Some(error),
            detail,
        },
        None => EmaError::UpstreamUnavailable {
            step: EmaStep::Redemption,
            detail,
        },
    }
}

fn oauth_error_from_body(body: &Value, fallback: &str) -> (Option<String>, String) {
    if let Some(code) = body.get("error").and_then(Value::as_str) {
        let detail = body
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or(code);
        return (Some(code.to_owned()), detail.to_owned());
    }
    if let Some(errors) = body.get("errors").and_then(Value::as_array) {
        for entry in errors {
            if let Some(text) = entry.as_str()
                && let Some(code) = rfc6749_prefix(text)
            {
                return (Some(code.to_owned()), text.to_owned());
            }
        }
    }
    (None, fallback.to_owned())
}

fn rfc6749_prefix(text: &str) -> Option<&'static str> {
    const CODES: &[&str] = &[
        "invalid_request",
        "invalid_client",
        "invalid_grant",
        "unauthorized_client",
        "unsupported_grant_type",
        "invalid_scope",
        "invalid_target",
    ];
    let trimmed = text.trim();
    CODES.iter().copied().find(|code| {
        trimmed == *code
            || trimmed
                .strip_prefix(code)
                .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with(':'))
    })
}

fn insert_well_known(origin: &str, suffix: &str, path: Option<&str>) -> String {
    path.map_or_else(
        || format!("{origin}/.well-known/{suffix}"),
        |path| format!("{origin}/.well-known/{suffix}{path}"),
    )
}

async fn fetch_metadata(
    url: &str,
    timeout: Duration,
) -> Result<Option<AuthorizationServerMetadata>, EmaError> {
    let response = match http().get(url).timeout(timeout).send().await {
        Ok(r) => r,
        Err(e) => {
            return Err(EmaError::UpstreamUnavailable {
                step: EmaStep::TokenExchange,
                detail: e.to_string(),
            });
        }
    };
    let status = response.status();
    if !status.is_success() {
        return Ok(None);
    }
    let body: Value = response.json().await.unwrap_or_else(|_| json!({}));
    let issuer = body.get("issuer").and_then(Value::as_str);
    let token_endpoint = body.get("token_endpoint").and_then(Value::as_str);
    let (Some(issuer), Some(token_endpoint)) = (issuer, token_endpoint) else {
        return Ok(None);
    };
    let profiles = body
        .get("authorization_grant_profiles_supported")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(AuthorizationServerMetadata {
        issuer: issuer.to_owned(),
        token_endpoint: token_endpoint.to_owned(),
        authorization_grant_profiles_supported: profiles,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        AuthorizationServerMetadata, EmaMintInput, discover_authorization_server_metadata,
        mint_enterprise_managed_access_token, resource_metadata_urls,
        run_enterprise_managed_authorization, try_fill_token,
    };
    use executor_core::{
        AuthMethod, EmaError, ID_JAG_GRANT_PROFILE, ID_JAG_TOKEN_TYPE, ID_JAG_TOKEN_TYPE_SENTINEL,
        JWT_BEARER_GRANT_TYPE,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::Duration;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mint<'a>(idp: &'a str, resource: &'a str, audience: &'a str) -> EmaMintInput<'a> {
        EmaMintInput {
            idp_token_url: idp,
            idp_client_id: "mcp-client-at-idp",
            idp_client_secret: None,
            subject_token: "sub-tok",
            subject_token_type: "urn:ietf:params:oauth:token-type:access_token",
            resource_token_url: resource,
            audience,
            resource_client_id: "mcp-client-at-resource",
            resource_client_secret: None,
            resource: None,
            scopes: &[],
        }
    }

    #[test]
    fn prm_urls_are_path_then_origin() {
        let urls = resource_metadata_urls("https://mcp.example/v1/sse");
        assert_eq!(
            urls,
            vec![
                "https://mcp.example/.well-known/oauth-protected-resource/v1/sse".to_owned(),
                "https://mcp.example/.well-known/oauth-protected-resource".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn mints_access_token_from_identity_assertion() {
        let idp = MockServer::start().await;
        let resource = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("token-exchange"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "id-jag.jwt",
                "issued_token_type": ID_JAG_TOKEN_TYPE,
                "token_type": ID_JAG_TOKEN_TYPE_SENTINEL,
                "scope": "mcp.read mcp.write",
            })))
            .mount(&idp)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("jwt-bearer"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "mcp-at",
                "token_type": "Bearer",
                "scope": "mcp.read mcp.write",
            })))
            .mount(&resource)
            .await;
        let grant = mint_enterprise_managed_access_token(
            &mint(
                &format!("{}/token", idp.uri()),
                &format!("{}/token", resource.uri()),
                &resource.uri(),
            ),
            Duration::from_secs(5),
        )
        .await
        .expect("mint");
        assert_eq!(grant.access_token, "mcp-at");
        assert_eq!(grant.scope, "mcp.read mcp.write");
    }

    #[tokio::test]
    async fn unsupported_profile_is_the_fallback() {
        let meta = AuthorizationServerMetadata {
            issuer: "https://as.example".into(),
            token_endpoint: "https://as.example/token".into(),
            authorization_grant_profiles_supported: vec![JWT_BEARER_GRANT_TYPE.into()],
        };
        let err = run_enterprise_managed_authorization(
            mint(
                "https://idp/token",
                "https://as.example/token",
                "https://as.example",
            ),
            &meta,
            Duration::from_secs(1),
        )
        .await
        .expect_err("unsupported");
        assert!(matches!(err, EmaError::GrantProfileUnsupported { .. }));
        assert!(err.may_fallback());
    }

    #[tokio::test]
    async fn policy_denied_does_not_fallback() {
        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "unauthorized_client",
                "error_description": "Policy does not permit this client",
            })))
            .mount(&idp)
            .await;
        let err = mint_enterprise_managed_access_token(
            &mint(
                &format!("{}/token", idp.uri()),
                "https://as.example/token",
                "https://as.example",
            ),
            Duration::from_secs(5),
        )
        .await
        .expect_err("denied");
        assert!(
            matches!(err, EmaError::PolicyDenied { ref error, .. } if error == "unauthorized_client")
        );
        assert!(!err.may_fallback());
    }

    #[tokio::test]
    async fn dead_assertion_is_subject_token_rejected() {
        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "expired",
            })))
            .mount(&idp)
            .await;
        let err = mint_enterprise_managed_access_token(
            &mint(
                &format!("{}/token", idp.uri()),
                "https://as.example/token",
                "https://as.example",
            ),
            Duration::from_secs(5),
        )
        .await
        .expect_err("rejected");
        assert!(matches!(err, EmaError::SubjectTokenRejected { .. }));
        assert!(!err.may_fallback());
    }

    #[tokio::test]
    async fn bearer_token_type_is_not_an_id_jag() {
        let idp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "not-a-jag",
                "issued_token_type": ID_JAG_TOKEN_TYPE,
                "token_type": "Bearer",
            })))
            .mount(&idp)
            .await;
        let err = mint_enterprise_managed_access_token(
            &mint(
                &format!("{}/token", idp.uri()),
                "https://as.example/token",
                "https://as.example",
            ),
            Duration::from_secs(5),
        )
        .await
        .expect_err("sentinel");
        assert!(matches!(err, EmaError::UpstreamUnavailable { .. }));
    }

    #[tokio::test]
    async fn idp_narrowed_scope_is_what_redemption_asks_for() {
        let idp = MockServer::start().await;
        let resource = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "id-jag.jwt",
                "issued_token_type": ID_JAG_TOKEN_TYPE,
                "token_type": ID_JAG_TOKEN_TYPE_SENTINEL,
                "scope": "mcp.read",
            })))
            .mount(&idp)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("scope=mcp.read"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "narrow-at",
                "token_type": "Bearer",
                "scope": "mcp.read",
            })))
            .mount(&resource)
            .await;
        let scopes = ["mcp.read".into(), "mcp.write".into()];
        let idp_token = format!("{}/token", idp.uri());
        let resource_token = format!("{}/token", resource.uri());
        let audience = resource.uri();
        let mut input = mint(&idp_token, &resource_token, &audience);
        input.scopes = &scopes;
        let grant = mint_enterprise_managed_access_token(&input, Duration::from_secs(5))
            .await
            .expect("mint");
        assert_eq!(grant.scope, "mcp.read");
    }

    #[tokio::test]
    async fn discovery_reads_grant_profile() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": server.uri(),
                "authorization_endpoint": format!("{}/authorize", server.uri()),
                "token_endpoint": format!("{}/token", server.uri()),
                "authorization_grant_profiles_supported": [ID_JAG_GRANT_PROFILE],
            })))
            .mount(&server)
            .await;
        let meta = discover_authorization_server_metadata(&server.uri(), Duration::from_secs(5))
            .await
            .expect("discover")
            .expect("present");
        assert_eq!(
            meta.authorization_grant_profiles_supported,
            [ID_JAG_GRANT_PROFILE]
        );
    }

    #[tokio::test]
    async fn prepare_fills_token_when_profile_is_advertised() {
        let idp = MockServer::start().await;
        let resource = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": resource.uri(),
                "authorization_endpoint": format!("{}/authorize", resource.uri()),
                "token_endpoint": format!("{}/token", resource.uri()),
                "authorization_grant_profiles_supported": [ID_JAG_GRANT_PROFILE],
            })))
            .mount(&resource)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("token-exchange"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "id-jag.jwt",
                "issued_token_type": ID_JAG_TOKEN_TYPE,
                "token_type": ID_JAG_TOKEN_TYPE_SENTINEL,
            })))
            .mount(&idp)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("jwt-bearer"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "from-ema",
                "token_type": "Bearer",
            })))
            .mount(&resource)
            .await;
        let methods = AuthMethod::oauth(
            format!("{}/authorize", resource.uri()),
            format!("{}/token", resource.uri()),
            vec!["mcp.read".into()],
        );
        let mut values = BTreeMap::from([
            ("identity_assertion".into(), "sub-tok".into()),
            ("idp_token_url".into(), format!("{}/token", idp.uri())),
            ("idp_client_id".into(), "mcp-client-at-idp".into()),
            ("audience".into(), resource.uri()),
            ("client_id".into(), "mcp-client-at-resource".into()),
        ]);
        let filled = try_fill_token(&methods, &mut values, Duration::from_secs(5))
            .await
            .expect("ema");
        assert!(filled);
        assert_eq!(values.get("token").map(String::as_str), Some("from-ema"));
    }
}
