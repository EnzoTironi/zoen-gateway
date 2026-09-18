//! Treg catalog, `/call`, balance, team tools, console bootstrap.

use std::collections::BTreeMap;

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use executor_catalog::{CallInput, CatalogError};
use executor_core::{KV_ARENA_VOTES, LOCAL_SUBJECT};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;

/// Catalog / call / balance / team-tools / bootstrap.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/catalog", get(search_catalog))
        .route("/api/catalog/{*id}", get(get_catalog))
        .route("/api/call", post(call_catalog))
        .route("/api/integrations/browse", get(browse_integrations))
        .route("/api/balance", get(get_balance).post(topup_balance))
        .route("/api/balance/grant", post(topup_balance))
        .route("/api/team-tools", get(list_team_tools).post(add_team_tool))
        .route(
            "/api/overflow-relays",
            get(list_overflow).post(add_overflow),
        )
        .route("/api/overflow", get(get_overflow).post(set_overflow))
        .route("/api/arena/capabilities", get(arena_capabilities))
        .route("/api/arena/run", post(arena_run))
        .route("/api/arena/votes", get(arena_votes).post(arena_vote))
        .route(
            "/call/{*url}",
            get(faithful_relay)
                .post(faithful_relay)
                .put(faithful_relay)
                .patch(faithful_relay)
                .delete(faithful_relay),
        )
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
        "catalog_size": state.catalog.catalog().all().len(),
        "providers": state
            .catalog
            .catalog()
            .providers()
            .into_iter()
            .map(|(slug, endpoints)| json!({ "slug": slug, "endpoints": endpoints }))
            .collect::<Vec<_>>(),
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

const GOOGLE_PRESETS: &[(&str, &str, &str, &str)] = &[
    (
        "google-gmail",
        "Gmail",
        "Mensagens, threads, labels e rascunhos.",
        "https://www.googleapis.com/discovery/v1/apis/gmail/v1/rest",
    ),
    (
        "google-calendar",
        "Google Calendar",
        "Calendários, eventos, ACLs e agendamento.",
        "https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest",
    ),
    (
        "google-drive",
        "Google Drive",
        "Arquivos, pastas, permissões e drives compartilhados.",
        "https://www.googleapis.com/discovery/v1/apis/drive/v3/rest",
    ),
    (
        "google-sheets",
        "Google Sheets",
        "Planilhas, valores, intervalos e formatação.",
        "https://www.googleapis.com/discovery/v1/apis/sheets/v4/rest",
    ),
    (
        "google-docs",
        "Google Docs",
        "Documentos, edições estruturais e formatação.",
        "https://www.googleapis.com/discovery/v1/apis/docs/v1/rest",
    ),
    (
        "google-slides",
        "Google Slides",
        "Apresentações, slides e elementos de página.",
        "https://www.googleapis.com/discovery/v1/apis/slides/v1/rest",
    ),
    (
        "google-chat",
        "Google Chat",
        "Espaços, mensagens, membros e reações.",
        "https://www.googleapis.com/discovery/v1/apis/chat/v1/rest",
    ),
    (
        "google-tasks",
        "Google Tasks",
        "Listas de tarefas, itens e prazos.",
        "https://www.googleapis.com/discovery/v1/apis/tasks/v1/rest",
    ),
];

async fn browse_integrations(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "plugins": [
            {
                "key": "openapi",
                "name": "OpenAPI / Swagger",
                "summary": "Registre uma spec HTTP (URL ou JSON). Inclui Google Discovery.",
            },
            {
                "key": "graphql",
                "name": "GraphQL",
                "summary": "Endpoint GraphQL com schema opcional.",
            },
            {
                "key": "mcp",
                "name": "MCP",
                "summary": "Servidor MCP HTTP ou comando stdio.",
            },
        ],
        "google": GOOGLE_PRESETS.iter().map(|(id, name, summary, url)| {
            json!({ "id": id, "name": name, "summary": summary, "url": url, "kind": "google-discovery" })
        }).collect::<Vec<_>>(),
        "providers": state
            .catalog
            .catalog()
            .providers()
            .into_iter()
            .map(|(slug, endpoints)| json!({ "slug": slug, "endpoints": endpoints }))
            .collect::<Vec<_>>(),
    }))
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
        Err(err) => catalog_error(&err),
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
        Err(e) => catalog_error(&e),
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
        Err(e) => catalog_error(&e),
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
    let origin = state.public_origin.trim_end_matches('/');
    Json(json!({
        "origin": origin,
        "mcp_url": format!("{origin}/mcp"),
        "token": state.auth_token,
        "locale": "pt-BR",
        "product": "executor-treg",
        "catalog_size": state.catalog.catalog().all().len(),
    }))
}

async fn list_overflow(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "relays": state.catalog.list_overflow_relays() }))
}

#[derive(Deserialize)]
struct OverflowBody {
    name: String,
    provider: String,
    base_url: String,
    secret: String,
}

async fn add_overflow(
    State(state): State<AppState>,
    Json(body): Json<OverflowBody>,
) -> impl IntoResponse {
    let tool =
        state
            .catalog
            .register_overflow_relay(body.name, body.provider, body.base_url, body.secret);
    (StatusCode::CREATED, Json(tool)).into_response()
}

#[derive(Deserialize)]
struct OverflowFlag {
    #[serde(default)]
    opt_out: bool,
}

async fn get_overflow(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "opt_out": state.catalog.overflow_opt_out(),
        "relays": state.catalog.list_overflow_relays(),
    }))
}

async fn set_overflow(
    State(state): State<AppState>,
    Json(body): Json<OverflowFlag>,
) -> impl IntoResponse {
    state.catalog.set_overflow_opt_out(body.opt_out);
    Json(json!({ "opt_out": body.opt_out }))
}

async fn arena_capabilities(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "capabilities": state.catalog.arena_capabilities() }))
}

#[derive(Deserialize)]
struct ArenaRunBody {
    capability: String,
    #[serde(default)]
    query: BTreeMap<String, String>,
}

async fn arena_run(
    State(state): State<AppState>,
    Json(body): Json<ArenaRunBody>,
) -> impl IntoResponse {
    let endpoints = state.catalog.endpoints_for_capability(&body.capability);
    if endpoints.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            format!("capacidade não encontrada: {}", body.capability),
        )
            .into_response();
    }
    let mut results = Vec::new();
    for endpoint in endpoints {
        let started = std::time::Instant::now();
        let connection_secret =
            crate::connections::inject_secret(&state.executor, &endpoint.provider);
        let outcome = state
            .catalog
            .call(CallInput {
                id: endpoint.id.clone(),
                query: body.query.clone(),
                body: None,
                subject: LOCAL_SUBJECT.to_owned(),
                connection_secret,
                team_tool_secret: None,
            })
            .await;
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match outcome {
            Ok(out) => results.push(json!({
                "id": endpoint.id,
                "provider": endpoint.provider,
                "name": endpoint.name,
                "status": out.status,
                "served_via": out.served_via,
                "cost_micro": out.cost_micro,
                "ms": ms,
                "body": out.body,
            })),
            Err(err) => results.push(json!({
                "id": endpoint.id,
                "provider": endpoint.provider,
                "name": endpoint.name,
                "error": err.to_string(),
                "ms": ms,
            })),
        }
    }
    Json(json!({
        "capability": body.capability,
        "results": results,
    }))
    .into_response()
}

async fn arena_votes(State(state): State<AppState>) -> impl IntoResponse {
    match state.executor.list_kv(KV_ARENA_VOTES) {
        Ok(votes) => Json(json!({ "votes": votes })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct ArenaVoteBody {
    capability: String,
    winner_id: String,
}

async fn arena_vote(
    State(state): State<AppState>,
    Json(body): Json<ArenaVoteBody>,
) -> impl IntoResponse {
    if body.capability.is_empty() || body.winner_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "capability e winner_id obrigatórios",
        )
            .into_response();
    }
    let count = if let Ok(Some(existing)) = state.executor.get_kv(KV_ARENA_VOTES, &body.capability)
        && existing.get("winner_id").and_then(Value::as_str) == Some(body.winner_id.as_str())
    {
        existing.get("count").and_then(Value::as_i64).unwrap_or(0) + 1
    } else {
        1_i64
    };
    let row = json!({
        "capability": body.capability,
        "winner_id": body.winner_id,
        "count": count,
    });
    if let Err(err) = state
        .executor
        .put_kv(KV_ARENA_VOTES, &body.capability, row.clone())
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    Json(row).into_response()
}

async fn faithful_relay(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    let uri = req.uri().clone();
    let path = uri.path().strip_prefix("/call/").unwrap_or("");
    let decoded = urlencoding_decode(path);
    let url = if let Some(query) = uri.query() {
        format!("{decoded}?{query}")
    } else {
        decoded
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return (StatusCode::BAD_REQUEST, "URL absoluta obrigatória").into_response();
    }
    let Some((tool, secret)) = state.catalog.team_tool_for_url(&url) else {
        return (
            StatusCode::NOT_FOUND,
            format!("nenhuma ferramenta da equipe cobre {url}"),
        )
            .into_response();
    };
    let method = req.method().clone();
    let headers = req.headers().clone();
    let body = axum::body::to_bytes(req.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(err) => return (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
    };
    let reqwest_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut upstream = client.request(reqwest_method, &url);
    for (name, value) in &headers {
        let key = name.as_str();
        if matches!(
            key,
            "authorization"
                | "host"
                | "content-length"
                | "x-treg-token"
                | "x-executor-token"
                | "cookie"
                | "connection"
                | "transfer-encoding"
        ) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            upstream = upstream.header(key, v);
        }
    }
    upstream = upstream.header("authorization", format!("Bearer {secret}"));
    if !body.is_empty() {
        upstream = upstream.body(body.to_vec());
    }
    match upstream.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let payload = resp.json::<Value>().await.unwrap_or_else(|_| json!({}));
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                Json(json!({
                    "served_via": "team_tool",
                    "tool": tool.id,
                    "body": payload,
                })),
            )
                .into_response()
        }
        Err(err) => (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
    }
}

fn urlencoding_decode(s: &str) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(char::from(v));
                i += 3;
                continue;
            }
        }
        out.push(char::from(bytes[i]));
        i += 1;
    }
    out
}

/// Map a catalog error to an HTTP response.
pub fn catalog_error(err: &CatalogError) -> axum::response::Response {
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
        CatalogError::ParameterInvalid(msg) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "catalog_parameter_invalid",
                "message": msg,
            })),
        )
            .into_response(),
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

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn browse_and_strict_query_and_policy() {
        let app = app();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/integrations/browse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert!(
            body["plugins"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["key"] == "openapi"),
            "{body}"
        );
        assert!(
            body["google"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["id"] == "google-gmail"),
            "{body}"
        );

        let bad = json!({
            "id": "demo.strict",
            "query": { "text": "olá", "extra": "nope" }
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/call")
                    .header("content-type", "application/json")
                    .body(Body::from(bad))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = json_body(res).await;
        assert_eq!(err["error"], "catalog_parameter_invalid");

        let ok = json!({
            "id": "demo.strict",
            "query": { "text": "olá" }
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/call")
                    .header("content-type", "application/json")
                    .body(Body::from(ok))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = json_body(res).await;
        assert_eq!(body["served_via"], "anonymous", "{body}");

        let policy = json!({
            "pattern": "demo.*",
            "action": "require_approval"
        })
        .to_string();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/policies")
                    .header("content-type", "application/json")
                    .body(Body::from(policy))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "{}", json_body(res).await);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/console/bootstrap")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let boot = json_body(res).await;
        assert_eq!(boot["locale"], "pt-BR");
        assert!(
            boot["mcp_url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/mcp")),
            "{boot}"
        );
    }

    #[allow(clippy::too_many_lines)]
    #[tokio::test]
    async fn orgs_topup_skills_arena_jail_and_spa() {
        let state = crate::AppState::new(Executor::builder().build(), None);
        assert!(state.catalog.register_cli("echo").is_ok());
        let app = crate::http::app(state, &Limits::production());

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("accept", "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let html = String::from_utf8(
            res.into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("<strong>executor</strong>"), "{html}");
        assert!(html.contains("class=\"stack\""), "{html}");

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/orgs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let orgs = json_body(res).await;
        assert_eq!(orgs["orgs"][0]["slug"], "local", "{orgs}");

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/orgs")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"name": "Acme"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let acme = json_body(res).await;
        assert_eq!(acme["slug"], "acme");

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/orgs/acme/invites")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"email": "ada@acme.com", "role": "admin"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let invite = json_body(res).await;
        let code = invite["code"].as_str().unwrap().to_owned();

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/orgs/join")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"code": code}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "{}", json_body(res).await);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/balance/topup")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({"micro": 2_000_000}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let topup = json_body(res).await;
        assert_eq!(topup["mode"], "local", "{topup}");
        let id = topup["id"].as_str().unwrap();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/balance/topup/{id}/confirm"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let paid = json_body(res).await;
        assert!(
            paid["balance_micro"].as_i64().unwrap() >= 3_000_000,
            "{paid}"
        );

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/skills")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"slug": "seo-blog", "body": "# SEO\n\nEscreva."}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/arena/capabilities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let caps = json_body(res).await;
        assert!(
            caps["capabilities"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["capability"] == "people.email.find"),
            "{caps}"
        );

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/arena/run")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "capability": "people.email.find",
                            "query": {"domain": "stripe.com", "full_name": "Ada"}
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let race = json_body(res).await;
        assert!(
            race["results"].as_array().is_some_and(|r| r.len() >= 2),
            "{race}"
        );

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cli/run")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"binary": "echo", "args": ["parity"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cli = json_body(res).await;
        assert!(
            cli["stdout"].as_str().is_some_and(|s| s.contains("parity")),
            "{cli}"
        );

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/cli/run")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"binary": "bash", "args": ["-c", "id"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/team-tools")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "stripe",
                            "provider": "stripe",
                            "base_url": "https://api.stripe.com",
                            "secret": "sk_test"
                        })
                        .to_string(),
                    ))
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

        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/artifacts")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"name": "nota.md", "body": "olá"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
}
