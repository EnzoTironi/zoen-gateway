//! Teams, invites, and memberships (Treg org surface).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_core::{KV_INVITES, KV_MEMBERSHIPS, KV_ORGS, LOCAL_SUBJECT};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;

const MAX_OWNED_ORGS: usize = 10;

/// Org HTTP group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/orgs", get(list_orgs).post(create_org))
        .route("/api/orgs/join", post(join_org))
        .route("/api/orgs/{slug}/invites", post(invite_member))
        .route("/api/orgs/{slug}/members", get(list_members))
}

#[derive(Deserialize)]
struct CreateOrg {
    name: String,
}

#[derive(Deserialize)]
struct InviteBody {
    email: String,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct JoinBody {
    code: String,
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

fn parse_role(role: Option<&str>) -> Result<&'static str, &'static str> {
    match role.unwrap_or("member") {
        "owner" => Ok("owner"),
        "admin" => Ok("admin"),
        "member" => Ok("member"),
        "viewer" => Ok("viewer"),
        _ => Err("papel deve ser owner, admin, member ou viewer"),
    }
}

fn ensure_local(state: &AppState) -> Result<(), String> {
    if state
        .executor
        .get_kv(KV_ORGS, "local")
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(());
    }
    state
        .executor
        .put_kv(
            KV_ORGS,
            "local",
            json!({
                "slug": "local",
                "name": "Local",
                "owner": LOCAL_SUBJECT,
            }),
        )
        .map_err(|e| e.to_string())?;
    state
        .executor
        .put_kv(
            KV_MEMBERSHIPS,
            &format!("local:{LOCAL_SUBJECT}"),
            json!({
                "org": "local",
                "subject": LOCAL_SUBJECT,
                "role": "owner",
                "email": LOCAL_SUBJECT,
            }),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn kv_err(err: &impl ToString) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}

async fn list_orgs(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(err) = ensure_local(&state) {
        return kv_err(&err);
    }
    match state.executor.list_kv(KV_ORGS) {
        Ok(orgs) => Json(json!({ "orgs": orgs, "active": "local" })).into_response(),
        Err(err) => kv_err(&err),
    }
}

async fn create_org(
    State(state): State<AppState>,
    Json(body): Json<CreateOrg>,
) -> impl IntoResponse {
    if let Err(err) = ensure_local(&state) {
        return kv_err(&err);
    }
    let slug = slugify(&body.name);
    if slug.is_empty() {
        return (StatusCode::BAD_REQUEST, "nome da equipe inválido").into_response();
    }
    match state.executor.get_kv(KV_ORGS, &slug) {
        Ok(Some(_)) => {
            return (StatusCode::CONFLICT, format!("equipe já existe: {slug}")).into_response();
        }
        Ok(None) => {}
        Err(err) => return kv_err(&err),
    }
    let owned = match state.executor.list_kv(KV_ORGS) {
        Ok(rows) => rows
            .iter()
            .filter(|row| row.get("owner").and_then(Value::as_str) == Some(LOCAL_SUBJECT))
            .count(),
        Err(err) => return kv_err(&err),
    };
    if owned >= MAX_OWNED_ORGS {
        return (StatusCode::BAD_REQUEST, "limite de 10 equipes próprias").into_response();
    }
    let org = json!({
        "slug": slug,
        "name": body.name,
        "owner": LOCAL_SUBJECT,
    });
    if let Err(err) = state.executor.put_kv(KV_ORGS, &slug, org.clone()) {
        return kv_err(&err);
    }
    if let Err(err) = state.executor.put_kv(
        KV_MEMBERSHIPS,
        &format!("{slug}:{LOCAL_SUBJECT}"),
        json!({
            "org": slug,
            "subject": LOCAL_SUBJECT,
            "role": "owner",
            "email": LOCAL_SUBJECT,
        }),
    ) {
        return kv_err(&err);
    }
    (StatusCode::CREATED, Json(org)).into_response()
}

async fn invite_member(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(body): Json<InviteBody>,
) -> impl IntoResponse {
    if let Err(err) = ensure_local(&state) {
        return kv_err(&err);
    }
    let role = match parse_role(body.role.as_deref()) {
        Ok(role) => role,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };
    match state.executor.get_kv(KV_ORGS, &slug) {
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("equipe não encontrada: {slug}"),
            )
                .into_response();
        }
        Ok(Some(_)) => {}
        Err(err) => return kv_err(&err),
    }
    if body.email.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "e-mail obrigatório").into_response();
    }
    let mut raw = [0u8; 4];
    if getrandom::getrandom(&mut raw).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "não gerou o convite").into_response();
    }
    let code = hex::encode(raw);
    let invite = json!({
        "code": code,
        "org": slug,
        "email": body.email.trim(),
        "role": role,
    });
    if let Err(err) = state.executor.put_kv(KV_INVITES, &code, invite.clone()) {
        return kv_err(&err);
    }
    (StatusCode::CREATED, Json(invite)).into_response()
}

async fn join_org(State(state): State<AppState>, Json(body): Json<JoinBody>) -> impl IntoResponse {
    if let Err(err) = ensure_local(&state) {
        return kv_err(&err);
    }
    let invite = match state.executor.get_kv(KV_INVITES, body.code.trim()) {
        Ok(Some(invite)) => invite,
        Ok(None) => return (StatusCode::NOT_FOUND, "convite inválido").into_response(),
        Err(err) => return kv_err(&err),
    };
    let Some(org) = invite.get("org").and_then(Value::as_str) else {
        return (StatusCode::BAD_REQUEST, "convite sem equipe").into_response();
    };
    let role = invite
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("member");
    let email = invite
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or(LOCAL_SUBJECT);
    let membership = json!({
        "org": org,
        "subject": email,
        "role": role,
        "email": email,
    });
    if let Err(err) = state.executor.put_kv(
        KV_MEMBERSHIPS,
        &format!("{org}:{email}"),
        membership.clone(),
    ) {
        return kv_err(&err);
    }
    let _ = state.executor.delete_kv(KV_INVITES, body.code.trim());
    Json(membership).into_response()
}

async fn list_members(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    if let Err(err) = ensure_local(&state) {
        return kv_err(&err);
    }
    match state.executor.list_kv(KV_MEMBERSHIPS) {
        Ok(rows) => {
            let members: Vec<Value> = rows
                .into_iter()
                .filter(|row| row.get("org").and_then(Value::as_str) == Some(slug.as_str()))
                .collect();
            Json(json!({ "members": members })).into_response()
        }
        Err(err) => kv_err(&err),
    }
}
