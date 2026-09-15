//! Axum routes: `/health`, `/api/health`, `/metrics`, Streamable HTTP `/mcp`, original `/executions`.

use std::time::Duration;

use axum::BoxError;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
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
use crate::mcp::{ElicitationMode, McpMode, McpOptions, as_sse, handle_jsonrpc_with, sse_ping};

/// Build the HTTP router with concurrency/timeout/body limits.
pub fn app(state: AppState, limits: &Limits) -> Router {
    let concurrency = usize::try_from(limits.max_in_flight).unwrap_or(256);
    let body = limits.max_arg_bytes.max(64 * 1024);
    let timeout = limits.execute_timeout;
    Router::new()
        .route("/health", get(health))
        .route("/api/health", get(api_health))
        .route("/metrics", get(metrics))
        .route("/mcp", get(mcp_get).post(mcp_post).delete(mcp_delete))
        .route(
            "/mcp/toolkits/{slug}",
            get(mcp_toolkit_get)
                .post(mcp_toolkit_post)
                .delete(mcp_delete),
        )
        .route("/executions", post(api_executions))
        .route("/api/executions", post(api_executions))
        .route("/executions/{execution_id}", get(api_get_execution))
        .route(
            "/executions/{execution_id}/resume",
            post(api_resume_execution),
        )
        .route(
            "/api/executions/{execution_id}/resume",
            post(api_resume_execution),
        )
        .route("/api/execute", post(api_execute))
        .route("/api/resume", post(api_resume))
        .route("/api/tools", get(api_tools))
        .route("/api/integrations", get(api_integrations))
        .route("/api/integrations/detect", post(api_detect))
        .route("/api/policies", get(api_policies))
        .route("/api/oauth/clients", get(api_oauth_clients))
        .route("/api/toolkits", get(api_toolkits).post(api_toolkit_create))
        .route(
            "/api/toolkits/{slug}",
            axum::routing::delete(api_toolkit_delete),
        )
        .merge(crate::auth::routes())
        .merge(crate::well_known::routes())
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

async fn api_health() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], "ok")
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

#[derive(Debug, Deserialize, Default)]
struct McpQuery {
    mode: Option<String>,
    elicitation_mode: Option<String>,
    search_tools: Option<String>,
}

fn mcp_opts(query: &McpQuery, toolkit: Option<executor_core::Toolkit>) -> McpOptions {
    McpOptions {
        mode: McpMode::parse(query.mode.as_deref()),
        elicitation: ElicitationMode::parse(query.elicitation_mode.as_deref()),
        search_tools: query
            .search_tools
            .as_deref()
            .is_some_and(|v| v == "true" || v == "1"),
        toolkit,
    }
}

fn wants_sse(headers: &HeaderMap) -> bool {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    accept.contains("text/event-stream") && !accept.contains("application/json")
}

fn session_id_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
}

fn resolve_opts(state: &AppState, headers: &HeaderMap, query: &McpQuery) -> McpOptions {
    if let Some(id) = session_id_of(headers)
        && let Some(opts) = state.mcp.get(&id)
    {
        return opts;
    }
    mcp_opts(query, None)
}

async fn mcp_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<McpQuery>,
) -> impl IntoResponse {
    let _ = (state, headers, query);
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        sse_ping(),
    )
}

#[instrument(skip(state, body))]
async fn mcp_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<McpQuery>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    mcp_dispatch(state, headers, query, None, body).await
}

async fn mcp_toolkit_get() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        sse_ping(),
    )
}

async fn mcp_toolkit_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Query(query): Query<McpQuery>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let toolkit = state.executor.toolkit_by_slug(&slug).ok().flatten();
    mcp_dispatch(state, headers, query, toolkit, body).await
}

async fn mcp_dispatch(
    state: AppState,
    headers: HeaderMap,
    query: McpQuery,
    toolkit: Option<executor_core::Toolkit>,
    body: Value,
) -> axum::response::Response {
    let mut opts = resolve_opts(&state, &headers, &query);
    if toolkit.is_some() {
        opts.toolkit = toolkit;
        if query.mode.is_none() {
            opts.mode = McpMode::Passthrough;
        }
    }
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let mut session = session_id_of(&headers);
    if method == "initialize" && session.is_none() {
        session = Some(state.mcp.create(opts.clone()));
    }
    let response = handle_jsonrpc_with(&state.executor, body, &opts).await;
    if response.is_null() {
        return StatusCode::ACCEPTED.into_response();
    }
    let mut resp = if wants_sse(&headers) {
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            as_sse(&response),
        )
            .into_response()
    } else {
        Json(response).into_response()
    };
    if let Some(id) = session
        && let Ok(value) = HeaderValue::from_str(&id)
    {
        resp.headers_mut().insert("mcp-session-id", value);
    }
    resp
}

async fn mcp_delete(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(id) = session_id_of(&headers) {
        state.mcp.delete(&id);
    }
    StatusCode::NO_CONTENT
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
struct ExecutionBody {
    code: String,
    #[serde(default, alias = "autoApprove")]
    auto_approve: bool,
}

async fn api_executions(
    State(state): State<AppState>,
    Json(body): Json<ExecutionBody>,
) -> impl IntoResponse {
    match state
        .executor
        .run_code(
            &body.code,
            ExecuteOptions {
                auto_approve: body.auto_approve,
                timeout: None,
                idempotency_key: None,
            },
        )
        .await
    {
        Ok(outcome) => Json(outcome.execution_api()).into_response(),
        Err(err) => map_outcome(Err(err)),
    }
}

async fn api_get_execution(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> impl IntoResponse {
    let id = match ExecutionId::new(&execution_id) {
        Ok(id) => id,
        Err(e) => return err_status(StatusCode::BAD_REQUEST, e.to_string()),
    };
    match state.executor.get_execution(&id) {
        Ok(Some(state)) => {
            Json(json!({"executionId": execution_id, "state": state})).into_response()
        }
        Ok(None) => err_status(StatusCode::NOT_FOUND, "execution not found"),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct ResumeBody {
    execution_id: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    content: Option<Value>,
}

async fn api_resume(
    State(state): State<AppState>,
    Json(body): Json<ResumeBody>,
) -> impl IntoResponse {
    let _ = body.content;
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
struct ExecutionResumeBody {
    #[serde(default)]
    action: String,
    #[serde(default)]
    content: Option<Value>,
}

async fn api_resume_execution(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    Json(body): Json<ExecutionResumeBody>,
) -> impl IntoResponse {
    let _ = body.content;
    let id = match ExecutionId::new(&execution_id) {
        Ok(id) => id,
        Err(e) => return err_status(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let action = match body.action.as_str() {
        "" | "accept" => ResumeAction::Accept,
        "decline" => ResumeAction::Decline,
        "cancel" => ResumeAction::Cancel,
        other => return err_status(StatusCode::BAD_REQUEST, format!("unknown action {other}")),
    };
    match state.executor.resume(&id, action).await {
        Ok(outcome) => Json(outcome.execution_api()).into_response(),
        Err(err) => map_outcome(Err(err)),
    }
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

#[derive(Deserialize)]
struct DetectBody {
    url: String,
}

async fn api_detect(
    State(state): State<AppState>,
    Json(body): Json<DetectBody>,
) -> impl IntoResponse {
    Json(json!({ "results": state.executor.detect(&body.url) }))
}

async fn api_policies(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_policies() {
        Ok(rows) => Json(json!({ "policies": rows })).into_response(),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_oauth_clients(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_kv(executor_core::KV_OAUTH_CLIENTS) {
        Ok(clients) => Json(json!({ "clients": clients })).into_response(),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_toolkits(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_kv(executor_core::KV_TOOLKITS) {
        Ok(toolkits) => Json(json!({ "toolkits": toolkits })).into_response(),
        Err(e) => err_status(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_toolkit_create(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    map_outcome(
        state
            .executor
            .execute(
                "executor.coreTools.toolkits.create",
                body,
                ExecuteOptions {
                    auto_approve: true,
                    timeout: None,
                    idempotency_key: None,
                },
            )
            .await,
    )
}

async fn api_toolkit_delete(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    map_outcome(
        state
            .executor
            .execute(
                "executor.coreTools.toolkits.remove",
                json!({"slug": slug}),
                ExecuteOptions {
                    auto_approve: true,
                    timeout: None,
                    idempotency_key: None,
                },
            )
            .await,
    )
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
                ExecutorError::EnterpriseManaged(inner) => match inner {
                    executor_core::EmaError::PolicyDenied { .. } => StatusCode::FORBIDDEN,
                    executor_core::EmaError::SubjectTokenRejected { .. } => {
                        StatusCode::UNAUTHORIZED
                    }
                    executor_core::EmaError::UpstreamUnavailable { .. }
                    | executor_core::EmaError::GrantProfileUnsupported { .. }
                    | executor_core::EmaError::RedemptionRejected { .. } => StatusCode::BAD_GATEWAY,
                },
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
