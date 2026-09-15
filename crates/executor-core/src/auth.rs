//! Authentication templates. Core stores these as a closed enum, not stringly modes.

use serde::{Deserialize, Serialize};

/// Sentinel template slug for integrations that need no credential.
pub const NO_AUTH_TEMPLATE: &str = "none";

/// Slug of one declared auth method on an integration.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AuthTemplateSlug(String);

impl AuthTemplateSlug {
    /// Parse a non-empty template slug.
    ///
    /// # Errors
    ///
    /// Empty string.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, crate::InvalidId> {
        let value = raw.as_ref().trim();
        if value.is_empty() {
            return Err(crate::InvalidId::new(
                "auth template",
                raw.as_ref(),
                "must be non-empty",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// The no-auth template.
    #[must_use]
    pub fn none() -> Self {
        Self(NO_AUTH_TEMPLATE.to_owned())
    }

    /// Bearer-token template slug.
    #[must_use]
    pub fn bearer() -> Self {
        Self("bearer".to_owned())
    }

    /// API-key template slug.
    #[must_use]
    pub fn api_key() -> Self {
        Self("apiKey".to_owned())
    }

    /// Borrow the slug.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AuthTemplateSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a credential is applied on an outbound call.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Carrier {
    /// HTTP header.
    Header,
    /// Query string.
    Query,
    /// Child-process environment (stdio MCP servers).
    Env,
}

/// One rendering of a credential input onto a request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthPlacement {
    /// Carrier.
    pub carrier: Carrier,
    /// Header/query/env name.
    pub name: String,
    /// Literal prefix (`"Bearer "`), empty when bare.
    pub prefix: String,
    /// Input variable this placement reads (`token` by default).
    pub variable: String,
    /// When set, this placement is a static literal (no secret).
    pub literal: Option<String>,
}

impl AuthPlacement {
    /// Bearer header from the `token` variable.
    #[must_use]
    pub fn bearer_header() -> Self {
        Self {
            carrier: Carrier::Header,
            name: "Authorization".to_owned(),
            prefix: "Bearer ".to_owned(),
            variable: "token".to_owned(),
            literal: None,
        }
    }

    /// Named header, no prefix.
    #[must_use]
    pub fn header(name: impl Into<String>, variable: impl Into<String>) -> Self {
        Self {
            carrier: Carrier::Header,
            name: name.into(),
            prefix: String::new(),
            variable: variable.into(),
            literal: None,
        }
    }
}

/// Kind of auth method, closed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// No credential.
    None,
    /// API key / static secret via placements.
    ApiKey,
    /// Convenience for a single Authorization header.
    Header,
    /// OAuth access token (client-credentials or paused auth-code).
    Oauth,
}

/// Declared auth method on an integration catalog response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AuthMethod {
    /// Stable id within the integration (template slug).
    pub id: String,
    /// Human label.
    pub label: String,
    /// Kind.
    pub kind: AuthKind,
    /// Template slug a connection binds.
    pub template: AuthTemplateSlug,
    /// How to apply credential inputs.
    pub placements: Vec<AuthPlacement>,
    /// OAuth authorization URL, when kind is OAuth.
    pub authorization_url: Option<String>,
    /// OAuth token URL.
    pub token_url: Option<String>,
    /// OAuth scopes.
    pub scopes: Vec<String>,
}

impl AuthMethod {
    /// No credential.
    #[must_use]
    pub fn none() -> Self {
        Self {
            id: NO_AUTH_TEMPLATE.to_owned(),
            label: "None".to_owned(),
            kind: AuthKind::None,
            template: AuthTemplateSlug::none(),
            placements: Vec::new(),
            authorization_url: None,
            token_url: None,
            scopes: Vec::new(),
        }
    }

    /// Single `Authorization: Bearer` header from `token`.
    #[must_use]
    pub fn bearer() -> Self {
        Self {
            id: "bearer".to_owned(),
            label: "Bearer token".to_owned(),
            kind: AuthKind::Header,
            template: AuthTemplateSlug::bearer(),
            placements: vec![AuthPlacement::bearer_header()],
            authorization_url: None,
            token_url: None,
            scopes: Vec::new(),
        }
    }

    /// Named header API key (variable `token`).
    #[must_use]
    pub fn api_key_header(header_name: impl Into<String>) -> Self {
        Self {
            id: "apiKey".to_owned(),
            label: "API key".to_owned(),
            kind: AuthKind::ApiKey,
            template: AuthTemplateSlug::api_key(),
            placements: vec![AuthPlacement::header(header_name, "token")],
            authorization_url: None,
            token_url: None,
            scopes: Vec::new(),
        }
    }
}
