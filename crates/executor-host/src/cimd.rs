//! RFC 7591-adjacent Client ID Metadata Document for the local daemon.

use axum::extract::Path;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::origin_of;

/// CIMD routes (public).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/oauth/client-id-metadata.json", get(cimd_default))
        .route("/oauth/client-id-metadata/{target}", get(cimd_target))
}

fn document(origin: &str, path: &str, local: bool) -> Value {
    let client_id = format!("{origin}{path}");
    let callback = format!("{origin}/api/oauth/callback");
    let name = if local { "Executor Local" } else { "Executor" };
    json!({
        "client_id": client_id,
        "client_name": name,
        "client_uri": origin,
        "redirect_uris": [
            callback,
            "http://127.0.0.1/api/oauth/callback",
            "http://localhost/api/oauth/callback",
            "http://[::1]/api/oauth/callback"
        ],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": if local { "native" } else { "web" },
    })
}

async fn cimd_default(headers: HeaderMap) -> impl IntoResponse {
    let origin = origin_of(&headers);
    Json(document(&origin, "/oauth/client-id-metadata.json", false))
}

async fn cimd_target(headers: HeaderMap, Path(target): Path<String>) -> impl IntoResponse {
    let origin = origin_of(&headers);
    let target = target.trim_end_matches(".json");
    let local = target == "local";
    Json(document(
        &origin,
        &format!("/oauth/client-id-metadata/{target}.json"),
        local,
    ))
}

#[cfg(test)]
mod tests {
    use super::document;

    #[test]
    fn local_target_is_native() {
        let doc = document(
            "http://127.0.0.1:4788",
            "/oauth/client-id-metadata/local.json",
            true,
        );
        assert_eq!(doc["application_type"], "native");
        assert_eq!(doc["token_endpoint_auth_method"], "none");
        assert!(
            doc["redirect_uris"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        );
    }
}
