//! Named artifacts (Executor generated UI / notes).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use executor_core::KV_ARTIFACTS;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;

/// Artifacts HTTP group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/artifacts", get(list_artifacts).post(add_artifact))
        .route(
            "/api/artifacts/{id}",
            get(get_artifact).delete(delete_artifact),
        )
}

#[derive(Deserialize)]
struct ArtifactBody {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

async fn list_artifacts(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_kv(KV_ARTIFACTS) {
        Ok(artifacts) => Json(json!({ "artifacts": artifacts })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn add_artifact(
    State(state): State<AppState>,
    Json(body): Json<ArtifactBody>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "nome obrigatório").into_response();
    }
    let id = format!("art_{}", hex_id());
    let row = json!({
        "id": id,
        "name": body.name,
        "kind": body.kind.unwrap_or_else(|| "markdown".into()),
        "body": body.body.unwrap_or_default(),
    });
    if let Err(err) = state.executor.put_kv(KV_ARTIFACTS, &id, row.clone()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    (StatusCode::CREATED, Json(row)).into_response()
}

async fn get_artifact(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.executor.get_kv(KV_ARTIFACTS, &id) {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("artefato não encontrado: {id}"),
        )
            .into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn delete_artifact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.executor.delete_kv(KV_ARTIFACTS, &id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            format!("artefato não encontrado: {id}"),
        )
            .into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

fn hex_id() -> String {
    let mut raw = [0u8; 6];
    let _ = getrandom::getrandom(&mut raw);
    hex::encode(raw)
}
