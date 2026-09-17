//! Treg catalog, `/call`, balance, team tools, console bootstrap.

use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_catalog::{CallInput, CatalogError};
use executor_core::LOCAL_SUBJECT;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;

/// Catalog / call / balance / team-tools / bootstrap.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/catalog", get(search_catalog))
        .route("/api/catalog/{*id}", get(get_catalog))
        .route("/api/call", post(call_catalog))
        .route("/api/balance", get(get_balance).post(topup_balance))
        .route("/api/balance/grant", post(topup_balance))
        .route("/api/team-tools", get(list_team_tools).post(add_team_tool))
        .route("/api/console/bootstrap", get(bootstrap))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<usize>,
}

async fn search_catalog(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let hits = state
        .catalog
        .catalog()
        .search(q.q.as_deref().unwrap_or(""), q.limit.unwrap_or(24));
    Json(json!({
        "items": hits,
        "total": hits.len(),
    }))
}

async fn get_catalog(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    state.catalog.catalog().get(&id).map_or_else(
        || {
            (
                StatusCode::NOT_FOUND,
                format!("endpoint não encontrado: {id}"),
            )
                .into_response()
        },
        |ep| Json(ep).into_response(),
    )
}

#[derive(Deserialize)]
struct CallBody {
    id: String,
    #[serde(default)]
    query: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<Value>,
}

async fn call_catalog(
    State(state): State<AppState>,
    Json(body): Json<CallBody>,
) -> impl IntoResponse {
    let Some(endpoint) = state.catalog.catalog().get(&body.id).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            format!("endpoint não encontrado: {}", body.id),
        )
            .into_response();
    };
    let connection_secret = crate::connections::inject_secret(&state.executor, &endpoint.provider);
    match state
        .catalog
        .call(CallInput {
            id: body.id,
            query: body.query,
            body: body.body,
            subject: LOCAL_SUBJECT.to_owned(),
            connection_secret,
            team_tool_secret: None,
        })
        .await
    {
        Ok(out) => Json(out).into_response(),
        Err(err) => catalog_err(&err),
    }
}

async fn get_balance(State(state): State<AppState>) -> impl IntoResponse {
    match state.catalog.balance(LOCAL_SUBJECT) {
        Ok(balance_micro) => Json(json!({
            "subject": LOCAL_SUBJECT,
            "balance_micro": balance_micro,
            "currency": "USD",
            "topup_url": "/saldo",
        }))
        .into_response(),
        Err(e) => catalog_err(&e),
    }
}

#[derive(Deserialize)]
struct TopupBody {
    #[serde(default)]
    micro: Option<i64>,
}

async fn topup_balance(
    State(state): State<AppState>,
    Json(body): Json<TopupBody>,
) -> impl IntoResponse {
    match state
        .catalog
        .topup(LOCAL_SUBJECT, body.micro.unwrap_or(1_000_000))
    {
        Ok(balance_micro) => Json(json!({ "balance_micro": balance_micro })).into_response(),
        Err(e) => catalog_err(&e),
    }
}

async fn list_team_tools(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "tools": state.catalog.list_team_tools() }))
}

#[derive(Deserialize)]
struct TeamToolBody {
    name: String,
    provider: String,
    base_url: String,
    secret: String,
}

async fn add_team_tool(
    State(state): State<AppState>,
    Json(body): Json<TeamToolBody>,
) -> impl IntoResponse {
    let tool =
        state
            .catalog
            .register_team_tool(body.name, body.provider, body.base_url, body.secret);
    (StatusCode::CREATED, Json(tool)).into_response()
}

async fn bootstrap(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "origin": state.public_origin,
        "token": state.auth_token,
        "locale": "pt-BR",
        "product": "executor-treg",
    }))
}

fn catalog_err(err: &CatalogError) -> axum::response::Response {
    match err {
        CatalogError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            format!("endpoint não encontrado: {id}"),
        )
            .into_response(),
        CatalogError::ConnectYourKey { id, provider } => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "connect_your_key",
                "message": format!("conecte sua chave para {id} (provedor {provider})"),
                "id": id,
                "provider": provider,
            })),
        )
            .into_response(),
        CatalogError::PaymentRequired {
            balance_micro,
            estimated_cost_micro,
        } => (
            StatusCode::PAYMENT_REQUIRED,
            Json(json!({
                "error": "payment_required",
                "balance_micro": balance_micro,
                "estimated_cost_micro": estimated_cost_micro,
                "topup_url": "/saldo",
            })),
        )
            .into_response(),
        CatalogError::Invalid(msg) => (StatusCode::BAD_REQUEST, (*msg).to_string()).into_response(),
        CatalogError::InvalidDoc(msg) => (StatusCode::BAD_REQUEST, msg.clone()).into_response(),
        CatalogError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg.clone()).into_response(),
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

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn catalog_search_call_and_connection_ladder() {
        let app = app();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/catalog?q=encontrar%20e-mail")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert!(
            body["items"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|h| h["endpoint"]["id"]
                    .as_str()
                    .is_some_and(|id| id.contains("email.find"))),
            "{body}"
        );

        let call = json!({
            "id": "hunter.people.email.find",
            "query": {"domain": "stripe.com", "full_name": "Patrick Collison"}
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/call")
                    .header("content-type", "application/json")
                    .body(Body::from(call))
                    .unwrap(),
            )
            .await
            .unwrap();
        let billed = json_body(res).await;
        assert_eq!(billed["served_via"], "platform", "{billed}");
        assert_eq!(billed["cost_micro"], 10_000);

        let create = json!({
            "integration": "hunter",
            "name": "work",
            "template": "bearer",
            "values": {"token": "sk_own"}
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/connections")
                    .header("content-type", "application/json")
                    .body(Body::from(create))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::CREATED,
            "{}",
            json_body(res).await
        );

        let call = json!({
            "id": "hunter.people.email.find",
            "query": {"domain": "stripe.com", "full_name": "Patrick Collison"}
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/call")
                    .header("content-type", "application/json")
                    .body(Body::from(call))
                    .unwrap(),
            )
            .await
            .unwrap();
        let own = json_body(res).await;
        assert_eq!(own["served_via"], "connection", "{own}");
        assert_eq!(own["cost_micro"], 0);

        let refuse = json!({"id": "internal.private.crm"}).to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/call")
                    .header("content-type", "application/json")
                    .body(Body::from(refuse))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let err = json_body(res).await;
        assert_eq!(err["error"], "connect_your_key");

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/connections")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let listed = json_body(res).await;
        assert_eq!(listed["connections"][0]["integration"], "hunter");
        assert!(listed["connections"][0].get("values").is_none());
    }
}
