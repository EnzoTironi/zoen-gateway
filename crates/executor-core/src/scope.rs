//! Strict RFC 6750 / Google `ErrorInfo` `insufficient_scope` detection.
//!
//! A false positive strips re-authenticate recovery from a 403 that re-auth
//! would fix, so matching is structural: quoted WWW-Authenticate params, exact
//! JSON field values, never prose.

use serde_json::Value;

use crate::ToolError;
use crate::www_authenticate::parse_challenges;

/// Tool error code for a grant that does not cover the operation.
pub const OAUTH_SCOPE_INSUFFICIENT: &str = "oauth_scope_insufficient";

const MAX_DEPTH: u32 = 8;

/// Scopes the upstream named as required, when it named any.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsufficientScope {
    /// RFC 6750 `scope` attribute, split on whitespace. Empty when unnamed.
    pub required_scopes: Vec<String>,
}

impl InsufficientScope {
    /// Domain tool failure. Retry will not help; re-running the same OAuth grant
    /// will not either.
    #[must_use]
    pub fn into_tool_error(self, status: u16, details: Value) -> ToolError {
        let message = if self.required_scopes.is_empty() {
            "The access token does not cover the required scope".to_owned()
        } else {
            format!(
                "The access token does not cover the required scope: {}",
                self.required_scopes.join(" ")
            )
        };
        ToolError {
            code: OAUTH_SCOPE_INSUFFICIENT.into(),
            message,
            status: Some(status),
            details: Some(details),
            retryable: Some(false),
        }
    }
}

/// Inspect an upstream 401/403 body and headers. `None` means fall through.
#[must_use]
pub fn detect_insufficient_scope(
    body: Option<&Value>,
    headers: &[(impl AsRef<str>, impl AsRef<str>)],
) -> Option<InsufficientScope> {
    for (name, value) in headers {
        if !name.as_ref().eq_ignore_ascii_case("www-authenticate") {
            continue;
        }
        if let Some(detected) = detect_from_challenge(value.as_ref()) {
            return Some(detected);
        }
    }
    if body.is_some_and(detect_from_body) {
        Some(InsufficientScope {
            required_scopes: Vec::new(),
        })
    } else {
        None
    }
}

/// Classify an HTTP error: `oauth_scope_insufficient` or `http_error`.
#[must_use]
pub fn tool_error_from_http(status: u16, headers: &[(String, String)], body: &Value) -> ToolError {
    if let Some(detected) = detect_insufficient_scope(Some(body), headers) {
        let details = serde_json::json!({
            "requiredScopes": detected.required_scopes,
            "upstream": body,
        });
        return detected.into_tool_error(status, details);
    }
    ToolError {
        code: "http_error".into(),
        message: format!("HTTP {status}"),
        status: Some(status),
        details: Some(body.clone()),
        retryable: Some((500..600).contains(&status)),
    }
}

fn detect_from_challenge(header: &str) -> Option<InsufficientScope> {
    let challenges = parse_challenges(header)?;
    for challenge in challenges {
        if challenge.scheme != "bearer" {
            continue;
        }
        if challenge.params.get("error").map(String::as_str) != Some("insufficient_scope") {
            continue;
        }
        let required_scopes = challenge
            .params
            .get("scope")
            .map(|s| s.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_default();
        return Some(InsufficientScope { required_scopes });
    }
    None
}

fn detect_from_body(body: &Value) -> bool {
    match body {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .is_ok_and(|parsed| detect_from_structured(&parsed, 0)),
        other => detect_from_structured(other, 0),
    }
}

fn detect_from_structured(body: &Value, depth: u32) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match body {
        Value::Array(items) => items
            .iter()
            .any(|item| detect_from_structured(item, depth + 1)),
        Value::Object(map) => {
            if map.get("error").and_then(Value::as_str) == Some("insufficient_scope") {
                return true;
            }
            if map.get("reason").and_then(Value::as_str) == Some("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
            {
                return true;
            }
            map.values().any(|value| {
                (value.is_object() || value.is_array()) && detect_from_structured(value, depth + 1)
            })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{InsufficientScope, detect_insufficient_scope};
    use serde_json::json;

    fn headers(value: &str) -> Vec<(String, String)> {
        vec![("www-authenticate".into(), value.into())]
    }

    fn detected(
        body: Option<&serde_json::Value>,
        header: Option<&str>,
    ) -> Option<InsufficientScope> {
        let hdrs: Vec<(String, String)> = header.map(headers).unwrap_or_default();
        detect_insufficient_scope(body, &hdrs)
    }

    fn scopes(required: &[&str]) -> InsufficientScope {
        InsufficientScope {
            required_scopes: required.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn google_error_info() {
        let body = json!({
            "error": {
                "code": 403,
                "message": "Request had insufficient authentication scopes.",
                "status": "PERMISSION_DENIED",
                "details": [{
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
                    "domain": "googleapis.com",
                    "metadata": { "service": "drive.googleapis.com" }
                }]
            }
        });
        assert_eq!(detected(Some(&body), None), Some(scopes(&[])));
    }

    #[test]
    fn rfc6750_error_body() {
        assert_eq!(
            detected(Some(&json!({"error":"insufficient_scope"})), None),
            Some(scopes(&[]))
        );
    }

    #[test]
    fn challenge_scope_list() {
        assert_eq!(
            detected(
                None,
                Some(
                    r#"Bearer realm="example", error="insufficient_scope", scope="files.read files.meta""#
                ),
            ),
            Some(scopes(&["files.read", "files.meta"]))
        );
    }

    #[test]
    fn json_text_body() {
        assert_eq!(
            detected(Some(&json!("{\"error\":\"insufficient_scope\"}")), None),
            Some(scopes(&[]))
        );
    }

    #[test]
    fn non_json_text_never_classifies() {
        assert_eq!(
            detected(
                Some(&json!(
                    "Proxy note: error=insufficient_scope was returned upstream"
                )),
                None
            ),
            None
        );
        assert_eq!(
            detected(
                Some(&json!(
                    r#"The docs show {"error":"insufficient_scope"} as an example response"#
                )),
                None
            ),
            None
        );
    }

    #[test]
    fn quoted_description_does_not_count() {
        assert_eq!(
            detected(
                None,
                Some(
                    r#"Bearer error_description="Example: error=insufficient_scope for missing grants""#
                ),
            ),
            None
        );
    }

    #[test]
    fn prose_object_misses() {
        let body = json!({
            "error": {
                "message": "If the token lacks access you may see insufficient_scope or ACCESS_TOKEN_SCOPE_INSUFFICIENT in provider docs"
            }
        });
        assert_eq!(detected(Some(&body), None), None);
        assert_eq!(
            detected(
                Some(&json!(
                    "Consult the OAuth guide about insufficient_scope errors"
                )),
                None
            ),
            None
        );
    }

    #[test]
    fn other_field_names_miss() {
        assert_eq!(
            detected(
                Some(&json!({"supportedErrors":["invalid_token","insufficient_scope"]})),
                None
            ),
            None
        );
        assert_eq!(
            detected(Some(&json!({"code":"insufficient_scope"})), None),
            None
        );
    }

    #[test]
    fn lookalike_challenge_params() {
        assert_eq!(
            detected(None, Some(r#"Bearer x-error="insufficient_scope""#)),
            None
        );
        assert_eq!(
            detected(None, Some(r#"Bearer error="insufficient_scope_extra""#)),
            None
        );
    }

    #[test]
    fn quoted_pairs_cannot_fabricate_param() {
        assert_eq!(
            detected(
                None,
                Some(r#"Bearer error_description="Example: \"error=insufficient_scope""#),
            ),
            None
        );
    }

    #[test]
    fn scheme_inside_quoted_value() {
        assert_eq!(
            detected(
                None,
                Some(
                    r#"Basic error_description="Proxy saw Bearer error=insufficient_scope upstream""#
                ),
            ),
            None
        );
        assert_eq!(
            detected(
                None,
                Some(r#"Digest realm="x", qop="auth Bearer error=insufficient_scope", nonce="n""#),
            ),
            None
        );
        assert_eq!(
            detected(
                None,
                Some(r#"Basic error=insufficient_scope, Bearer realm="api""#)
            ),
            None
        );
    }

    #[test]
    fn only_bearer_params() {
        assert_eq!(
            detected(
                None,
                Some(r#"Basic realm="example", error=insufficient_scope"#)
            ),
            None
        );
        assert_eq!(
            detected(
                None,
                Some(r#"Bearer realm="api", Basic error=insufficient_scope"#),
            ),
            None
        );
    }

    #[test]
    fn repeated_bearer_independent() {
        assert_eq!(
            detected(
                None,
                Some("Bearer error=invalid_token, Basic realm=x, Bearer error=insufficient_scope"),
            ),
            Some(scopes(&[]))
        );
        assert_eq!(
            detected(
                None,
                Some(r#"Bearer scope="other.scope", Bearer error=insufficient_scope"#),
            ),
            Some(scopes(&[]))
        );
    }

    #[test]
    fn scheme_only_at_comma_boundary() {
        assert_eq!(
            detected(None, Some("Basic realm=x Bearer error=insufficient_scope")),
            None
        );
        assert_eq!(
            detected(
                None,
                Some("Basic error_description=Proxy Bearer error=insufficient_scope"),
            ),
            None
        );
    }

    #[test]
    fn token68_then_bearer() {
        assert_eq!(
            detected(
                None,
                Some("Negotiate abc/def==, Bearer error=insufficient_scope"),
            ),
            Some(scopes(&[]))
        );
    }

    #[test]
    fn malformed_headers() {
        assert_eq!(
            detected(None, Some(r#"Bearer error="insufficient_scope"#)),
            None
        );
        assert_eq!(
            detected(
                None,
                Some(r#"Basic realm="x"Bearer error=insufficient_scope"#)
            ),
            None
        );
    }

    #[test]
    fn bws_around_equals() {
        for header in [
            "Bearer error =insufficient_scope",
            "Bearer error= insufficient_scope",
            r#"Bearer error = "insufficient_scope""#,
        ] {
            assert_eq!(detected(None, Some(header)), Some(scopes(&[])), "{header}");
        }
    }

    #[test]
    fn params_after_token68_or_scheme_only() {
        for header in [
            "Bearer a=, error=insufficient_scope",
            "Bearer abc, error=insufficient_scope",
            "Bearer, error=insufficient_scope",
            r#"Bearer realm="x" error=insufficient_scope"#,
        ] {
            assert_eq!(detected(None, Some(header)), None, "{header}");
        }
    }

    #[test]
    fn http_token_characters() {
        for header in [
            "Bearer foo!=bar, error=insufficient_scope",
            "Bearer x#=bar, error=insufficient_scope",
            "Bearer x|=bar, error=insufficient_scope",
            "Foo! realm=x, Bearer error=insufficient_scope",
        ] {
            assert_eq!(detected(None, Some(header)), Some(scopes(&[])), "{header}");
        }
    }

    #[test]
    fn empty_non_token_duplicate_signal() {
        for header in [
            "Bearer realm =, error=insufficient_scope",
            "Bearer realm=;, error=insufficient_scope",
            "Bearer error=insufficient_scope, error=invalid_token",
            "Bearer foo/bar=baz, error=insufficient_scope",
            "Foo/Bar realm=x, Bearer error=insufficient_scope",
            "Bearer realm=foo/bar, error=insufficient_scope",
            "Bearer realm=foo=bar, error=insufficient_scope",
            "Bearer abc!==, error=insufficient_scope",
        ] {
            assert_eq!(detected(None, Some(header)), None, "{header}");
        }
    }

    #[test]
    fn provider_quirks() {
        assert_eq!(
            detected(
                None,
                Some(
                    "Bearer resource_metadata=https://mcp.stripe.com/.well-known/oauth-protected-resource, error=insufficient_scope"
                ),
            ),
            Some(scopes(&[]))
        );
        assert_eq!(
            detected(
                None,
                Some(
                    r#"Bearer realm="OAuth", resource_metadata="https://mcp.sentry.dev/.well-known/oauth-protected-resource", error="insufficient_scope", resource_metadata="https://mcp.sentry.dev/.well-known/oauth-protected-resource""#
                ),
            ),
            Some(scopes(&[]))
        );
        assert_eq!(
            detected(
                None,
                Some("Bearer error=insufficient_scope, error=invalid_token"),
            ),
            None
        );
    }

    #[test]
    fn comma_separated_params() {
        assert_eq!(
            detected(
                None,
                Some(r#"Bearer realm="api", error="insufficient_scope", scope="a.b""#),
            ),
            Some(scopes(&["a.b"]))
        );
    }

    #[test]
    fn unquoted_rfc6750() {
        assert_eq!(
            detected(None, Some("Bearer error=insufficient_scope")),
            Some(scopes(&[]))
        );
    }

    #[test]
    fn ordinary_403() {
        let body = json!({
            "error": { "status": "PERMISSION_DENIED", "message": "Caller lacks permission" }
        });
        assert_eq!(
            detected(
                Some(&body),
                Some(r#"Bearer realm="example", error="invalid_token""#)
            ),
            None
        );
    }

    #[test]
    fn empty_input() {
        let empty: Vec<(String, String)> = Vec::new();
        assert_eq!(detect_insufficient_scope(None, &empty), None);
    }
}
