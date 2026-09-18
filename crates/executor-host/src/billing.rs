//! Prepaid top-up: local confirm or Stripe Checkout.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use executor_core::LOCAL_SUBJECT;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;

/// Top-up + webhook HTTP group.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/balance/topup", post(create_topup))
        .route("/api/balance/topup/{id}/confirm", post(confirm_topup))
        .route("/api/stripe/webhook", post(stripe_webhook))
}

#[derive(Deserialize)]
struct TopupBody {
    #[serde(default)]
    micro: Option<i64>,
}

async fn create_topup(
    State(state): State<AppState>,
    Json(body): Json<TopupBody>,
) -> impl IntoResponse {
    let micro = body.micro.unwrap_or(5_000_000);
    let intent = match state.catalog.create_topup(LOCAL_SUBJECT, micro) {
        Ok(intent) => intent,
        Err(err) => return crate::catalog_api::catalog_error(&err),
    };
    let origin = state.public_origin.trim_end_matches('/');
    if let Ok(key) = std::env::var("STRIPE_SECRET_KEY")
        && !key.is_empty()
    {
        match stripe_checkout(&key, &intent.id, intent.amount_micro, origin).await {
            Ok(url) => {
                return Json(json!({
                    "mode": "stripe",
                    "id": intent.id,
                    "amount_micro": intent.amount_micro,
                    "checkout_url": url,
                }))
                .into_response();
            }
            Err(err) => {
                return (StatusCode::BAD_GATEWAY, err).into_response();
            }
        }
    }
    Json(json!({
        "mode": "local",
        "id": intent.id,
        "amount_micro": intent.amount_micro,
        "checkout_url": format!("{origin}/saldo?topup={}", intent.id),
        "confirm_url": format!("/api/balance/topup/{}/confirm", intent.id),
    }))
    .into_response()
}

async fn confirm_topup(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.catalog.confirm_topup(&id) {
        Ok(balance_micro) => Json(json!({
            "id": id,
            "balance_micro": balance_micro,
            "mode": "local",
        }))
        .into_response(),
        Err(err) => crate::catalog_api::catalog_error(&err),
    }
}

async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if let Ok(secret) = std::env::var("STRIPE_WEBHOOK_SECRET")
        && !secret.is_empty()
    {
        let sig = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !sig.contains(secret.as_str()) {
            return (StatusCode::UNAUTHORIZED, "assinatura Stripe inválida").into_response();
        }
    }
    let parsed: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
    let event_type = parsed.get("type").and_then(Value::as_str).unwrap_or("");
    if event_type != "checkout.session.completed" && event_type != "topup.confirmed" {
        return Json(json!({ "ok": true, "ignored": event_type })).into_response();
    }
    let id = parsed
        .pointer("/data/object/client_reference_id")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("id").and_then(Value::as_str));
    let Some(id) = id else {
        return (StatusCode::BAD_REQUEST, "client_reference_id ausente").into_response();
    };
    match state.catalog.confirm_topup(id) {
        Ok(balance_micro) => {
            Json(json!({ "ok": true, "balance_micro": balance_micro })).into_response()
        }
        Err(err) => crate::catalog_api::catalog_error(&err),
    }
}

async fn stripe_checkout(
    secret: &str,
    id: &str,
    micro: i64,
    origin: &str,
) -> Result<String, String> {
    let cents = (micro / 10_000).max(50);
    let params = [
        ("mode", "payment".to_owned()),
        ("success_url", format!("{origin}/saldo?paid={id}")),
        ("cancel_url", format!("{origin}/saldo?cancel={id}")),
        ("client_reference_id", id.to_owned()),
        ("line_items[0][quantity]", "1".into()),
        ("line_items[0][price_data][currency]", "usd".into()),
        ("line_items[0][price_data][unit_amount]", cents.to_string()),
        (
            "line_items[0][price_data][product_data][name]",
            "Executor crédito".into(),
        ),
    ];
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .basic_auth(secret, None::<&str>)
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body = resp.json::<Value>().await.map_err(|e| e.to_string())?;
    body.get("url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            body.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Stripe não criou a sessão")
                .to_owned()
        })
}
