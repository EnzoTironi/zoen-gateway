//! Plugin HTTP groups: `OpenAPI` / GraphQL / MCP config plane (not jiti factories).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_core::{ExecuteOptions, IntegrationSlug};
use serde_json::{Value, json};

use crate::AppState;
use crate::http::json_outcome;

/// First-party plugin HTTP groups.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/openapi/preview", post(openapi_preview))
        .route("/openapi/specs", post(openapi_add))
        .route(
            "/openapi/integrations/{slug}",
            get(openapi_get).delete(openapi_remove),
        )
        .route(
            "/openapi/integrations/{slug}/config",
            get(integration_config).post(openapi_configure),
        )
        .route("/openapi/integrations/{slug}/spec", post(openapi_update))
        .route("/graphql/integrations", post(graphql_add))
        .route("/graphql/integrations/{slug}", get(openapi_get))
        .route(
            "/graphql/integrations/{slug}/config",
            get(integration_config).post(graphql_configure),
        )
        .route("/mcp/probe", post(mcp_probe))
        .route("/mcp/servers", post(mcp_add))
        .route(
            "/mcp/servers/{slug}",
            get(mcp_get).delete(mcp_remove).post(mcp_configure),
        )
        .route("/mcp/servers/{slug}/config", post(mcp_configure))
        .route("/mcp/servers/{slug}/auth", post(mcp_auth))
}

fn yes() -> ExecuteOptions {
    ExecuteOptions {
        auto_approve: true,
        ..ExecuteOptions::default()
    }
}

fn unwrap_spec(body: &Value) -> Value {
    let spec = body.get("spec").cloned().unwrap_or(Value::Null);
    match spec {
        Value::Object(map) if map.get("kind").and_then(Value::as_str) == Some("url") => {
            json!({ "url": map.get("url") })
        }
        Value::Object(map) if map.get("kind").and_then(Value::as_str) == Some("blob") => map
            .get("value")
            .cloned()
            .or_else(|| map.get("blob").cloned())
            .unwrap_or(Value::Object(map)),
        other => other,
    }
}

async fn openapi_preview(Json(body): Json<Value>) -> impl IntoResponse {
    let spec = unwrap_spec(&body);
    let name = spec
        .pointer("/info/title")
        .and_then(Value::as_str)
        .or_else(|| body.get("name").and_then(Value::as_str))
        .unwrap_or("spec");
    Json(json!({
        "preview": true,
        "name": name,
        "slug": body.get("slug"),
    }))
}

async fn openapi_add(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    let mut args = body.clone();
    if let Some(obj) = args.as_object_mut() {
        obj.insert("spec".into(), unwrap_spec(&body));
    }
    json_outcome(
        &state,
        state
            .executor
            .execute("executor.openapi.addSpec", args, yes())
            .await,
    )
}

async fn openapi_get(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    integration_view(&state, &slug)
}

async fn openapi_remove(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    json_outcome(
        &state,
        state
            .executor
            .execute(
                "executor.coreTools.integrations.remove",
                json!({ "slug": slug }),
                yes(),
            )
            .await,
    )
}

async fn integration_config(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let parsed = match IntegrationSlug::new(&slug) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    match state.executor.get_integration_record(&parsed) {
        Ok(Some(row)) => Json(row.config).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn openapi_configure(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    patch_config(&state, &slug, &body, false)
}

async fn graphql_configure(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    patch_config(&state, &slug, &body, false)
}

async fn openapi_update(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut patch = body.clone();
    if let Some(obj) = patch.as_object_mut()
        && obj.contains_key("spec")
    {
        obj.insert("spec".into(), unwrap_spec(&body));
    }
    patch_config(&state, &slug, &patch, true)
}

async fn graphql_add(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    json_outcome(
        &state,
        state
            .executor
            .execute("executor.graphql.addIntegration", body, yes())
            .await,
    )
}

async fn mcp_probe(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    let url = body
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Json(json!({ "results": state.executor.detect(url) }))
}

async fn mcp_add(State(state): State<AppState>, Json(body): Json<Value>) -> impl IntoResponse {
    json_outcome(
        &state,
        state
            .executor
            .execute("executor.mcp.addServer", body, yes())
            .await,
    )
}

async fn mcp_get(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    integration_view(&state, &slug)
}

async fn mcp_remove(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    json_outcome(
        &state,
        state
            .executor
            .execute(
                "executor.coreTools.integrations.remove",
                json!({ "slug": slug }),
                yes(),
            )
            .await,
    )
}

async fn mcp_configure(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let patch = body.get("config").cloned().unwrap_or(body);
    patch_config(&state, &slug, &patch, false)
}

async fn mcp_auth(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    patch_config(&state, &slug, &body, false)
}

fn integration_view(state: &AppState, slug: &str) -> axum::response::Response {
    let parsed = match IntegrationSlug::new(slug) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    match state.executor.get_integration_record(&parsed) {
        Ok(Some(row)) => Json(json!({
            "slug": row.integration.slug.as_str(),
            "description": row.integration.description,
            "kind": row.integration.kind.as_str(),
            "canRemove": row.integration.can_remove,
            "canRefresh": row.integration.can_refresh,
            "config": row.config,
        }))
        .into_response(),
        Ok(None) => Json(Value::Null).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

fn patch_config(
    state: &AppState,
    slug: &str,
    patch: &Value,
    refresh: bool,
) -> axum::response::Response {
    let parsed = match IntegrationSlug::new(slug) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let mut row = match state.executor.get_integration_record(&parsed) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    merge_config(&mut row.config, patch);
    if let Err(e) = state.executor.put_integration_record(row.clone()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response();
    }
    if refresh && let Ok(conns) = state.executor.list_connections(Some(&parsed), None) {
        for conn in conns {
            let id = executor_core::ConnectionRef {
                owner: conn.owner,
                name: conn.name,
                integration: conn.integration,
            };
            let exec = state.executor.clone();
            tokio::spawn(async move {
                let _ = exec.refresh_connection(&id).await;
            });
        }
    }
    Json(json!({
        "slug": parsed.as_str(),
        "config": row.config,
    }))
    .into_response()
}

fn merge_config(config: &mut Value, patch: &Value) {
    let Some(patch_obj) = patch.as_object() else {
        return;
    };
    if !config.is_object() {
        *config = json!({});
    }
    let Some(config_obj) = config.as_object_mut() else {
        return;
    };
    let mode = patch_obj
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("merge");
    for (key, value) in patch_obj {
        if key == "mode" {
            continue;
        }
        if key == "authenticationTemplate" && mode == "merge" {
            let mut existing = config_obj
                .get("authenticationTemplate")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(extra) = value.as_array() {
                existing.extend(extra.iter().cloned());
            }
            config_obj.insert(key.clone(), Value::Array(existing));
            continue;
        }
        config_obj.insert(key.clone(), value.clone());
    }
}
