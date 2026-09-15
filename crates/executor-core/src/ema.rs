//! MCP Enterprise-Managed Authorization (ID-JAG) types.
//!
//! Protocol: MCP EMA profiles + `draft-ietf-oauth-identity-assertion-authz-grant-04`.
//! The engine crate runs the two-step grant. This module owns the constants,
//! the error taxonomy, and the RFC 8414 advertisement gate.
//!
//! `GrantProfileUnsupported` is the **only** failure that MAY fall back to
//! interactive OAuth. `PolicyDenied` and `SubjectTokenRejected` MUST NOT.

use std::fmt::{Display, Formatter, Result as FmtResult};

/// draft-ietf-oauth-identity-assertion-authz-grant-04 §7.2 profile identifier.
pub const ID_JAG_GRANT_PROFILE: &str = "urn:ietf:params:oauth:grant-profile:id-jag";

/// RFC 8693 §2.1 token-exchange grant.
pub const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";

/// RFC 7523 §2.1 JWT bearer grant — how an ID-JAG is redeemed (draft §4.4).
pub const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// draft §4.3 `requested_token_type` / §4.3.4 `issued_token_type`.
pub const ID_JAG_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";

/// draft §4.3.4: an ID-JAG is not an access token, so `token_type` is this sentinel.
pub const ID_JAG_TOKEN_TYPE_SENTINEL: &str = "N_A";

/// Default `subject_token_type` when the connection does not name one.
pub const DEFAULT_SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

/// JSON key for persisted EMA wiring inside connection `provider_state`.
pub const ENTERPRISE_MANAGED_PROVIDER_STATE_KEY: &str = "enterpriseManaged";

/// Which token-endpoint hop failed when the upstream was unreachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmaStep {
    /// RFC 8693 exchange at the enterprise `IdP`.
    TokenExchange,
    /// RFC 7523 redemption at the Resource Authorization Server.
    Redemption,
}

impl EmaStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::TokenExchange => "token-exchange",
            Self::Redemption => "redemption",
        }
    }
}

impl Display for EmaStep {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(self.as_str())
    }
}

/// Failures of the ID-JAG chain. Distinctions are load-bearing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmaError {
    /// Resource AS does not advertise the ID-JAG profile. MAY fall back.
    GrantProfileUnsupported {
        /// RFC 8414 `issuer`.
        issuer: String,
        /// `authorization_grant_profiles_supported` as advertised (may be empty).
        advertised: Vec<String>,
    },
    /// `IdP` refused to mint an ID-JAG. MUST NOT fall back.
    PolicyDenied {
        /// RFC 6749 §5.2 code (`unauthorized_client`, `access_denied`, …).
        error: String,
        /// Redacted human detail.
        detail: String,
    },
    /// Identity assertion expired, revoked, or bound to another client. MUST NOT fall back.
    SubjectTokenRejected {
        /// Redacted human detail.
        detail: String,
    },
    /// Resource AS refused the ID-JAG (wrong `typ`/`aud`/`client_id`, bad sig, expiry).
    RedemptionRejected {
        /// RFC 6749 §5.2 code when present.
        error: Option<String>,
        /// Redacted human detail.
        detail: String,
    },
    /// Transport / timeout / non-OAuth answer. Retryable; not an authorization verdict.
    UpstreamUnavailable {
        /// Which hop failed.
        step: EmaStep,
        /// Redacted human detail.
        detail: String,
    },
}

impl EmaError {
    /// Only [`Self::GrantProfileUnsupported`] may fall through to authorization-code.
    #[must_use]
    pub const fn may_fallback(&self) -> bool {
        matches!(self, Self::GrantProfileUnsupported { .. })
    }
}

impl Display for EmaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::GrantProfileUnsupported { issuer, advertised } => {
                write!(
                    f,
                    "The authorization server {issuer} does not advertise {ID_JAG_GRANT_PROFILE}"
                )?;
                if !advertised.is_empty() {
                    write!(f, " (advertised: {})", advertised.join(", "))?;
                }
                Ok(())
            }
            Self::PolicyDenied { error, detail } => write!(
                f,
                "Your organization's identity provider did not authorize this MCP server ({error}): {detail}"
            ),
            Self::SubjectTokenRejected { detail } => write!(
                f,
                "The enterprise identity assertion was rejected and a new single sign-on is required: {detail}"
            ),
            Self::RedemptionRejected { error, detail } => {
                write!(
                    f,
                    "The MCP server's authorization server rejected the identity assertion grant"
                )?;
                if let Some(code) = error {
                    write!(f, " ({code})")?;
                }
                write!(f, ": {detail}")
            }
            Self::UpstreamUnavailable { step, detail } => write!(
                f,
                "The enterprise-managed authorization {step} request failed: {detail}"
            ),
        }
    }
}

impl std::error::Error for EmaError {}

/// Whether RFC 8414 metadata advertises the ID-JAG grant profile (§7.2).
///
/// This is the **only** discovery signal that gates EMA. `grant_types_supported`
/// containing `jwt-bearer` is deliberately ignored: that grant predates this
/// profile and says nothing about ID-JAG processing rules.
#[must_use]
pub fn supports_id_jag_grant_profile(profiles: &[String]) -> bool {
    profiles.iter().any(|p| p == ID_JAG_GRANT_PROFILE)
}

#[cfg(test)]
mod tests {
    use super::{
        EmaError, ID_JAG_GRANT_PROFILE, JWT_BEARER_GRANT_TYPE, supports_id_jag_grant_profile,
    };

    #[test]
    fn jwt_bearer_grant_is_not_the_profile_gate() {
        assert!(!supports_id_jag_grant_profile(&[
            JWT_BEARER_GRANT_TYPE.into()
        ]));
        assert!(supports_id_jag_grant_profile(
            &[ID_JAG_GRANT_PROFILE.into()]
        ));
    }

    #[test]
    fn only_unsupported_profile_may_fallback() {
        let unsupported = EmaError::GrantProfileUnsupported {
            issuer: "https://as.example".into(),
            advertised: vec![],
        };
        assert!(unsupported.may_fallback());
        assert!(
            !EmaError::PolicyDenied {
                error: "unauthorized_client".into(),
                detail: "no".into(),
            }
            .may_fallback()
        );
        assert!(
            !EmaError::SubjectTokenRejected {
                detail: "dead".into(),
            }
            .may_fallback()
        );
    }
}
