//! Local RFC 8628 device-login and OAuth DCR/callback (no browser chrome).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_core::unix_now_ms;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;

const MAX_GRANTS: usize = 1024;
const GRANT_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct DeviceGrant {
    user_code: String,
    approved: bool,
    expires: Instant,
    access_token: Option<String>,
}

/// In-process device-code table. Bounded, TTL'd.
#[derive(Default)]
pub struct HostAuth {
    grants: Mutex<BTreeMap<String, DeviceGrant>>,
}

impl HostAuth {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn prune(grants: &mut BTreeMap<String, DeviceGrant>) {
        let now = Instant::now();
        grants.retain(|_, g| g.expires > now);
        while grants.len() > MAX_GRANTS {
            if let Some(key) = grants.keys().next().cloned() {
                grants.remove(&key);
            } else {
                break;
            }
        }
    }
}

/// Auth routes mounted on the daemon.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/cli-login", get(cli_login))
        .route("/api/auth/device/code", post(device_code))
        .route("/api/auth/device/token", post(device_token))
        .route("/api/auth/device/approve", post(device_approve))
        .route("/api/auth/device/verify", get(device_verify))
        .route("/api/oauth/register", post(oauth_register))
        .route("/api/oauth/callback", get(oauth_callback))
        .route("/api/execute-code", post(execute_code))
}

pub fn origin_of(headers: &HeaderMap) -> String {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:4788");
    format!("http://{host}")
}

async fn cli_login(headers: HeaderMap) -> impl IntoResponse {
    let origin = origin_of(&headers);
    Json(json!({
        "provider": "executor-local",
        "deviceAuthorizationEndpoint": format!("{origin}/api/auth/device/code"),
        "tokenEndpoint": format!("{origin}/api/auth/device/token"),
        "clientId": "executor-cli",
        "scope": "executor",
        "requestFormat": "json",
    }))
}

async fn device_code(State(state): State<AppState>) -> impl IntoResponse {
    let seq = unix_now_ms();
    let device_code = format!("dev_{seq:x}");
    let user_code = format!("{:08X}", seq % 0xFFFF_FFFF);
    let grant = DeviceGrant {
        user_code: user_code.clone(),
        approved: false,
        expires: Instant::now() + GRANT_TTL,
        access_token: None,
    };
    {
        let mut grants = state.auth.grants.lock();
        HostAuth::prune(&mut grants);
        grants.insert(device_code.clone(), grant);
    }
    Json(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": "http://127.0.0.1/device",
        "expires_in": 300,
        "interval": 1,
    }))
}

#[derive(Deserialize)]
struct TokenBody {
    #[serde(default)]
    grant_type: String,
    #[serde(default)]
    device_code: String,
}

#[allow(clippy::significant_drop_tightening)] // prune + lookup share one mutex
async fn device_token(
    State(state): State<AppState>,
    Json(body): Json<TokenBody>,
) -> impl IntoResponse {
    if body.grant_type != "urn:ietf:params:oauth:grant-type:device_code"
        && !body.grant_type.is_empty()
        && body.grant_type != "device_code"
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"unsupported_grant_type"})),
        )
            .into_response();
    }
    let outcome = {
        let mut grants = state.auth.grants.lock();
        HostAuth::prune(&mut grants);
        let Some(grant) = grants.get_mut(&body.device_code) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"expired_token"})),
            )
                .into_response();
        };
        if !grant.approved {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"authorization_pending"})),
            )
                .into_response();
        }
        grant
            .access_token
            .clone()
            .unwrap_or_else(|| format!("exec_{}", grant.user_code))
    };
    Json(json!({"access_token": outcome, "token_type": "Bearer"})).into_response()
}

#[derive(Deserialize)]
struct ApproveBody {
    user_code: String,
}

#[allow(clippy::significant_drop_tightening)] // lookup + mutate share one mutex
async fn device_approve(
    State(state): State<AppState>,
    Json(body): Json<ApproveBody>,
) -> impl IntoResponse {
    #[allow(clippy::significant_drop_tightening)]
    {
        let mut grants = state.auth.grants.lock();
        let Some(grant) = grants.values_mut().find(|g| g.user_code == body.user_code) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"unknown user_code"})),
            )
                .into_response();
        };
        grant.approved = true;
        grant.access_token = Some(format!("exec_{}", grant.user_code));
    }
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
struct VerifyQuery {
    user_code: Option<String>,
}

async fn device_verify(Query(q): Query<VerifyQuery>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "user_code": q.user_code,
        "message": "POST /api/auth/device/approve with this user_code (no UI).",
    }))
}

#[derive(Deserialize)]
struct DcrBody {
    registration_endpoint: String,
    redirect_uri: String,
}

async fn oauth_register(Json(body): Json<DcrBody>) -> impl IntoResponse {
    match executor_engine::register_client(
        &body.registration_endpoint,
        &body.redirect_uri,
        Duration::from_secs(15),
    )
    .await
    {
        Ok(client) => Json(json!({
            "client_id": client.client_id,
            "client_secret": client.client_secret,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn oauth_callback(Query(q): Query<CallbackQuery>) -> impl IntoResponse {
    Json(json!({
        "ok": q.error.is_none(),
        "code": q.code,
        "state": q.state,
        "error": q.error,
    }))
}

#[derive(Deserialize)]
struct CodeBody {
    source: String,
    #[serde(default)]
    auto_approve: bool,
}

async fn execute_code(
    State(state): State<AppState>,
    Json(body): Json<CodeBody>,
) -> impl IntoResponse {
    let result = state
        .executor
        .run_code(
            &body.source,
            executor_core::ExecuteOptions {
                auto_approve: body.auto_approve,
                timeout: None,
                idempotency_key: None,
            },
        )
        .await;
    match result {
        Ok(outcome) => Json(outcome.cli_json()).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": err.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use executor_core::Limits;
    use executor_engine::Executor;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn json_body(res: axum::response::Response) -> Value {
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
    }

    fn app() -> axum::Router {
        let state = crate::AppState::new(Executor::builder().build(), None);
        crate::http::app(state, &Limits::production())
    }

    #[tokio::test]
    async fn device_login_and_execute_code() {
        let app = app();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/cli-login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let doc = json_body(res).await;
        assert_eq!(doc["clientId"], "executor-cli");

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/device/code")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let grant = json_body(res).await;
        let user_code = grant["user_code"].as_str().expect("user_code").to_owned();
        let device_code = grant["device_code"]
            .as_str()
            .expect("device_code")
            .to_owned();

        let approve = json!({ "user_code": user_code }).to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/device/approve")
                    .header("content-type", "application/json")
                    .body(Body::from(approve))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let token_body = json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/device/token")
                    .header("content-type", "application/json")
                    .body(Body::from(token_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let tokens = json_body(res).await;
        assert!(tokens["access_token"].as_str().is_some(), "{tokens}");

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/execute-code")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"source":"return 1;"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let out = json_body(res).await;
        assert_eq!(out["status"], "completed");
        assert_eq!(out["result"]["data"], 1);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/oauth-authorization-server")
                    .header("host", "127.0.0.1:4788")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let meta = json_body(res).await;
        assert_eq!(meta["issuer"], "http://127.0.0.1:4788");
        assert!(meta.get("authorization_grant_profiles_supported").is_none());
    }
}
