//! Axum routes: `/health`, `/metrics`, `/mcp`, `/api/*`.

use std::time::Duration;

use axum::error_handling::HandleErrorLayer;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{BoxError, Json, Router};
use executor_core::{
    ExecuteOptions, ExecutionId, ExecutorError, IdempotencyKey, Limits, Outcome, ResumeAction,
    ToolListFilter, metric_names,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::instrument;

use crate::AppState;
use crate::mcp::handle_jsonrpc;

/// Build the HTTP router with concurrency/timeout/body limits.
pub fn app(state: AppState, limits: &Limits) -> Router {
    let concurrency = usize::try_from(limits.max_in_flight).unwrap_or(256);
    let body = limits.max_arg_bytes.max(64 * 1024);
    let timeout = limits.execute_timeout;
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/mcp", post(mcp_http))
        .route("/api/execute", post(api_execute))
        .route("/api/resume", post(api_resume))
        .route("/api/tools", get(api_tools))
        .route("/api/integrations", get(api_integrations))
        .merge(crate::auth::routes())
        .layer(DefaultBodyLimit::max(body))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: BoxError| async move {
                    layer_error(&err)
                }))
                .layer(TraceLayer::new_for_http())
                .layer(tower::timeout::TimeoutLayer::new(
                    timeout.saturating_add(Duration::from_secs(1)),
                ))
                .layer(tower::limit::ConcurrencyLimitLayer::new(concurrency)),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "plugins": state.executor.plugin_ids(),
    }))
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    #[allow(clippy::option_if_let_else)] // scrape payload vs noop exporter are different objects
    if let Some(m) = &state.metrics {
        let snap = m.snapshot();
        Json(json!({
            metric_names::EXECUTE_OK: snap.execute_ok,
            metric_names::EXECUTE_ERR: snap.execute_err,
            metric_names::EXECUTE_OVERLOAD: snap.execute_overload,
            metric_names::EXECUTE_TIMEOUT: snap.execute_timeout,
            metric_names::EXECUTE_CANCEL: snap.execute_cancel,
            metric_names::IN_FLIGHT: snap.in_flight,
            "executor.execute.ms": snap.last_execute_ms,
        }))
        .into_response()
    } else {
        Json(json!({"exporter":"noop"})).into_response()
    }
}

#[instrument(skip(state, body))]
async fn mcp_http(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    Json(handle_jsonrpc(&state.executor, body).await)
}

#[derive(Deserialize)]
struct ExecuteBody {
    path: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    auto_approve: bool,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[instrument(skip(state, body))]
async fn api_execute(
    State(state): State<AppState>,
    Json(body): Json<ExecuteBody>,
) -> impl IntoResponse {
    let key = match body.idempotency_key {
        Some(k) => match IdempotencyKey::new(k) {
            Ok(k) => Some(k),
            Err(e) => return err_status(StatusCode::BAD_REQUEST, e.to_string()),
        },
        None => None,
    };
    let args = if body.args.is_null() {
        json!({})
    } else {
        body.args
    };
    map_outcome(
        state
            .executor
            .execute(
                &body.path,
                args,
                ExecuteOptions {
                    auto_approve: body.auto_approve,
                    timeout: None,
                    idempotency_key: key,
                },
            )
            .await,
    )
}

#[derive(Deserialize)]
struct ResumeBody {
    execution_id: String,
    #[serde(default)]
    action: String,
}

async fn api_resume(
    State(state): State<AppState>,
    Json(body): Json<ResumeBody>,
) -> impl IntoResponse {
    let id = match ExecutionId::new(&body.execution_id) {
        Ok(id) => id,
        Err(e) => return err_status(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let action = match body.action.as_str() {
        "" | "accept" => ResumeAction::Accept,
        "decline" => ResumeAction::Decline,
        "cancel" => ResumeAction::Cancel,
        other => return err_status(StatusCode::BAD_REQUEST, format!("unknown action {other}")),
    };
    map_outcome(state.executor.resume(&id, action).await)
}

#[derive(Deserialize)]
struct ToolsQuery {
    q: Option<String>,
    include_blocked: Option<bool>,
}

async fn api_tools(
    State(state): State<AppState>,
    Query(query): Query<ToolsQuery>,
) -> impl IntoResponse {
    let filter = ToolListFilter {
        query: query.q,
        include_blocked: query.include_blocked.unwrap_or(false),
        ..ToolListFilter::default()
    };
    match state.executor.list_tools(&filter) {
        Ok(tools) => Json(json!({
            "tools": tools.iter().map(|t| json!({
                "path": t.cli_path(),
                "address": t.address.to_string(),
                "description": t.description,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_integrations(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_integrations() {
        Ok(rows) => Json(json!({
            "integrations": rows.iter().map(|i| json!({
                "slug": i.slug.as_str(),
                "name": i.name,
                "kind": i.kind.as_str(),
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn map_outcome(result: Result<Outcome, ExecutorError>) -> axum::response::Response {
    match result {
        Ok(outcome) => Json(outcome.cli_json()).into_response(),
        Err(err) => {
            let status = match &err {
                ExecutorError::Overloaded { .. } => StatusCode::TOO_MANY_REQUESTS,
                ExecutorError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
                ExecutorError::Cancelled => StatusCode::REQUEST_TIMEOUT,
                ExecutorError::ToolBlocked { .. } => StatusCode::FORBIDDEN,
                ExecutorError::ToolNotFound { .. } => StatusCode::NOT_FOUND,
                ExecutorError::InvalidArgs(_)
                | ExecutorError::InvalidPattern(_)
                | ExecutorError::InvalidId(_)
                | ExecutorError::Code(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::BAD_GATEWAY,
            };
            err_status(status, err.to_string())
        }
    }
}

fn layer_error(err: &BoxError) -> (StatusCode, Json<Value>) {
    if err.is::<tower::timeout::error::Elapsed>() {
        (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"error": "timeout"})),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": err.to_string()})),
        )
    }
}

fn err_status(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}
