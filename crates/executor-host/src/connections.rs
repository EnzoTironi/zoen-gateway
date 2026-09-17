//! First-class connections HTTP: list / create / get / delete / refresh.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_core::{
    AuthTemplateSlug, Connection, ConnectionInput, ConnectionName, ConnectionRef, ExecutorError,
    Integration, IntegrationRecord, IntegrationSlug, Owner, PluginId,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;

/// Connections REST group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/api/connections/{owner}/{integration}/{name}",
            get(get_connection)
                .delete(delete_connection)
                .patch(patch_connection),
        )
        .route(
            "/api/connections/{owner}/{integration}/{name}/refresh",
            post(refresh_connection),
        )
        .route(
            "/api/connections/{owner}/{integration}/{name}/validate",
            post(refresh_connection),
        )
        .route("/api/secrets", get(list_secrets))
}

fn public_connection(c: &Connection) -> Value {
    json!({
        "owner": c.owner.as_str(),
        "name": c.name.as_str(),
        "integration": c.integration.as_str(),
        "template": c.template.as_str(),
        "address": c.address.to_string(),
        "identity_label": c.identity_label,
        "description": c.description,
        "last_health": c.last_health,
    })
}

async fn list_connections(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_connections(None, None) {
        Ok(rows) => Json(json!({
            "connections": rows.iter().map(public_connection).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateBody {
    integration: String,
    name: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    identity_label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    values: BTreeMap<String, String>,
}

async fn create_connection(
    State(state): State<AppState>,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    let owner = body
        .owner
        .as_deref()
        .and_then(Owner::parse)
        .unwrap_or(Owner::Org);
    let template = match body.template.as_deref() {
        Some(t) => match AuthTemplateSlug::new(t) {
            Ok(t) => t,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        },
        None => AuthTemplateSlug::none(),
    };
    let name = match ConnectionName::new(&body.name) {
        Ok(n) => n,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let integration = match IntegrationSlug::new(&body.integration) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if let Err(e) = ensure_provider(&state, &integration) {
        return exec_err(&e);
    }
    match state
        .executor
        .create_connection(ConnectionInput {
            owner,
            name,
            integration,
            template,
            identity_label: body.identity_label,
            description: body.description,
            values: body.values,
            refs: BTreeMap::new(),
        })
        .await
    {
        Ok(conn) => (StatusCode::CREATED, Json(public_connection(&conn))).into_response(),
        Err(e) => exec_err(&e),
    }
}

fn exec_err(err: &ExecutorError) -> axum::response::Response {
    let status = match err {
        ExecutorError::Conflict(_) => StatusCode::CONFLICT,
        ExecutorError::InvalidArgs(_)
        | ExecutorError::InvalidId(_)
        | ExecutorError::InvalidPattern(_) => StatusCode::BAD_REQUEST,
        ExecutorError::IntegrationNotFound(_)
        | ExecutorError::ConnectionNotFound(_)
        | ExecutorError::ToolNotFound { .. }
        | ExecutorError::ExecutionNotFound(_) => StatusCode::NOT_FOUND,
        ExecutorError::ToolBlocked { .. } => StatusCode::FORBIDDEN,
        ExecutorError::Overloaded { .. } => StatusCode::TOO_MANY_REQUESTS,
        ExecutorError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
        ExecutorError::Cancelled => StatusCode::REQUEST_TIMEOUT,
        ExecutorError::PluginNotLoaded(_)
        | ExecutorError::ProviderNotRegistered(_)
        | ExecutorError::CredentialResolution(_)
        | ExecutorError::RemovalNotAllowed(_)
        | ExecutorError::Storage(_)
        | ExecutorError::Plugin(_)
        | ExecutorError::NotPaused(_)
        | ExecutorError::LimitExceeded(_)
        | ExecutorError::Code(_)
        | ExecutorError::EnterpriseManaged(_) => StatusCode::BAD_GATEWAY,
    };
    (status, err.to_string()).into_response()
}

async fn get_connection(
    State(state): State<AppState>,
    Path((owner, integration, name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match parse_ref(&owner, &integration, &name) {
        Ok(id) => match state.executor.get_connection(&id) {
            Ok(Some(c)) => Json(public_connection(&c)).into_response(),
            Ok(None) => StatusCode::NOT_FOUND.into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn delete_connection(
    State(state): State<AppState>,
    Path((owner, integration, name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match parse_ref(&owner, &integration, &name) {
        Ok(id) => match state.executor.remove_connection(&id) {
            Ok(true) => StatusCode::NO_CONTENT.into_response(),
            Ok(false) => StatusCode::NOT_FOUND.into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        },
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn refresh_connection(
    State(state): State<AppState>,
    Path((owner, integration, name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match parse_ref(&owner, &integration, &name) {
        Ok(id) => match state.executor.refresh_connection(&id).await {
            Ok(tools) => Json(json!({ "tools": tools.len() })).into_response(),
            Err(e) => exec_err(&e),
        },
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct PatchBody {
    #[serde(default)]
    identity_label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    values: BTreeMap<String, String>,
}

async fn patch_connection(
    State(state): State<AppState>,
    Path((owner, integration, name)): Path<(String, String, String)>,
    Json(body): Json<PatchBody>,
) -> impl IntoResponse {
    match parse_ref(&owner, &integration, &name) {
        Ok(id) => match state.executor.patch_connection(
            &id,
            body.identity_label,
            body.description,
            body.values,
        ) {
            Ok(conn) => Json(public_connection(&conn)).into_response(),
            Err(e) => exec_err(&e),
        },
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

fn parse_ref(owner: &str, integration: &str, name: &str) -> Result<ConnectionRef, String> {
    let owner = Owner::parse(owner).ok_or_else(|| "owner inválido".to_owned())?;
    let integration = IntegrationSlug::new(integration).map_err(|e| e.to_string())?;
    let name = ConnectionName::new(name).map_err(|e| e.to_string())?;
    Ok(ConnectionRef {
        owner,
        name,
        integration,
    })
}

async fn list_secrets(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_connections(None, None) {
        Ok(rows) => Json(json!({
            "secrets": rows.iter().map(|c| {
                json!({
                    "owner": c.owner.as_str(),
                    "integration": c.integration.as_str(),
                    "name": c.name.as_str(),
                    "keys": c.secrets.keys().cloned().collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Resolve an inject token for the first connection on `provider` (trusted space).
#[must_use]
pub fn inject_secret(executor: &executor_engine::Executor, provider: &str) -> Option<String> {
    let slug = IntegrationSlug::new(provider).ok()?;
    let rows = executor.list_connections(Some(&slug), None).ok()?;
    let conn = rows.into_iter().next()?;
    let id = ConnectionRef {
        owner: conn.owner,
        name: conn.name,
        integration: conn.integration,
    };
    executor.connection_inject_secret(&id).ok().flatten()
}

fn ensure_provider(state: &AppState, slug: &IntegrationSlug) -> Result<(), ExecutorError> {
    if state.executor.get_integration_record(slug)?.is_some() {
        return Ok(());
    }
    state.executor.put_integration_record(IntegrationRecord {
        integration: Integration {
            slug: slug.clone(),
            name: slug.as_str().to_owned(),
            description: "Provedor do catálogo Treg".into(),
            kind: PluginId::core_tools(),
            can_remove: true,
            can_refresh: false,
            auth_methods: Vec::new(),
            display_url: None,
        },
        config: json!({}),
    })
}
