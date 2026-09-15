//! RFC 9728 Protected Resource Metadata and RFC 8414 Authorization Server Metadata.
//!
//! This host is not an ID-JAG Resource Authorization Server. It MUST NOT
//! advertise `urn:ietf:params:oauth:grant-profile:id-jag`.

use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::AppState;

/// Well-known discovery routes (no UI).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/{*rest}",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        .route(
            "/.well-known/openid-configuration",
            get(authorization_server),
        )
}

async fn protected_resource(headers: HeaderMap) -> impl IntoResponse {
    let origin = crate::auth::origin_of(&headers);
    Json(json!({
        "resource": origin,
        "authorization_servers": [origin],
        "bearer_methods_supported": ["header"],
    }))
}

async fn authorization_server(headers: HeaderMap) -> impl IntoResponse {
    Json(authorization_server_metadata(&crate::auth::origin_of(
        &headers,
    )))
}

/// RFC 8414 document this daemon serves. Never includes the ID-JAG profile.
#[must_use]
pub fn authorization_server_metadata(origin: &str) -> Value {
    json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/api/oauth/callback"),
        "token_endpoint": format!("{origin}/api/auth/device/token"),
        "registration_endpoint": format!("{origin}/api/oauth/register"),
        "grant_types_supported": [
            "authorization_code",
            "refresh_token",
            "urn:ietf:params:oauth:grant-type:device_code"
        ],
        "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [],
    })
}

#[cfg(test)]
fn metadata_advertises_id_jag(doc: &Value) -> bool {
    doc.get("authorization_grant_profiles_supported")
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.iter()
                .any(|v| v.as_str() == Some(executor_core::ID_JAG_GRANT_PROFILE))
        })
}

#[cfg(test)]
mod tests {
    use super::{authorization_server_metadata, metadata_advertises_id_jag};
    use executor_core::ID_JAG_GRANT_PROFILE;

    #[test]
    fn host_does_not_advertise_id_jag() {
        let doc = authorization_server_metadata("http://127.0.0.1:4788");
        assert!(!metadata_advertises_id_jag(&doc));
        assert!(
            doc.get("authorization_grant_profiles_supported").is_none(),
            "{doc}"
        );
        let grants = doc["grant_types_supported"].as_array().expect("grants");
        assert!(
            !grants
                .iter()
                .any(|v| v.as_str() == Some(ID_JAG_GRANT_PROFILE)),
            "{doc}"
        );
    }
}
