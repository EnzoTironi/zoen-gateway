//! Uploaded `SKILL.md` bundles (Treg skill upload / install).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use executor_core::KV_SKILLS;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::skills::ExtraSkill;
use executor_engine::Executor;

/// Skills HTTP group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/skills", get(list_skills).post(add_skill))
        .route("/api/skills/{slug}", get(get_skill).delete(delete_skill))
        .route("/api/skills/{slug}/install", get(install_skill))
}

#[derive(Deserialize)]
struct SkillBody {
    slug: String,
    #[serde(default)]
    name: Option<String>,
    body: String,
    #[serde(default)]
    tools: Vec<String>,
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if (c == ' ' || c == '-' || c == '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// Bundles stored for MCP `skills`.
#[must_use]
pub fn extra_skills(executor: &Executor) -> Vec<ExtraSkill> {
    executor
        .list_kv(KV_SKILLS)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| {
            let name = row.get("slug").and_then(Value::as_str)?.to_owned();
            let summary = row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(name.as_str())
                .to_owned();
            let body = row.get("body").and_then(Value::as_str)?.to_owned();
            Some(ExtraSkill {
                name,
                summary,
                body,
            })
        })
        .collect()
}

async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_kv(KV_SKILLS) {
        Ok(skills) => Json(json!({ "skills": skills })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn add_skill(
    State(state): State<AppState>,
    Json(body): Json<SkillBody>,
) -> impl IntoResponse {
    let slug = slugify(&body.slug);
    if slug.is_empty() || body.body.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "slug e SKILL.md são obrigatórios").into_response();
    }
    let row = json!({
        "slug": slug,
        "name": body.name.unwrap_or_else(|| slug.clone()),
        "body": body.body,
        "tools": body.tools,
    });
    if let Err(err) = state.executor.put_kv(KV_SKILLS, &slug, row.clone()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    (StatusCode::CREATED, Json(row)).into_response()
}

async fn get_skill(State(state): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    match state.executor.get_kv(KV_SKILLS, &slug) {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("skill não encontrada: {slug}"),
        )
            .into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn delete_skill(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    match state.executor.delete_kv(KV_SKILLS, &slug) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            format!("skill não encontrada: {slug}"),
        )
            .into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn install_skill(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    match state.executor.get_kv(KV_SKILLS, &slug) {
        Ok(Some(row)) => {
            let body = row.get("body").and_then(Value::as_str).unwrap_or("");
            Json(json!({
                "slug": slug,
                "files": [{
                    "path": format!(".claude/skills/{slug}/SKILL.md"),
                    "body": body,
                }],
            }))
            .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("skill não encontrada: {slug}"),
        )
            .into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}
