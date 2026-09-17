//! Treg-style endpoint catalog: search by job, prices, credential ladder.
//!
//! Does not vendor the Python tree. Seed JSON ships in-tree; YAML ingest reads
//! Treg-shaped documents when the operator points at a directory.

#![allow(clippy::module_name_repetitions)]

mod call;
mod money;

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use call::{CallInput, CallOutcome, CatalogService, ServedVia, TeamTool};
pub use money::{MemoryLedger, SIGNUP_GRANT_MICRO};

/// How an endpoint may be served without a team connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// No provider key; free.
    Anonymous,
    /// Platform key, metered, only if `cost_micro` is published.
    #[default]
    Platform,
    /// Refuse unless the team supplies a connection or team-tool.
    OwnKeyOnly,
}

/// One catalogued endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoint {
    /// Stable id (`hunter.people.email.find`).
    pub id: String,
    /// Job capability (`people.email.find`).
    pub capability: String,
    /// Provider slug (matches an Executor integration when connected).
    pub provider: String,
    /// Display name.
    pub name: String,
    /// One-line summary.
    pub summary: String,
    /// Phrases for job search (pt-BR and en).
    #[serde(default)]
    pub jobs: Vec<String>,
    /// HTTP method.
    pub method: String,
    /// URL path.
    pub path: String,
    /// Origin, `mock://…`, or `$ENV`.
    pub base_url: String,
    /// Access rung without an own key.
    pub access: Access,
    /// Published price in micro-USD. `None` means unpublished → refuse platform.
    #[serde(default)]
    pub cost_micro: Option<i64>,
    /// Query parameter schema (name → type/required/example).
    #[serde(default)]
    pub query: serde_json::Value,
    /// Routed capability child.
    #[serde(default)]
    pub routed_child: Option<String>,
}

/// Search hit.
#[derive(Clone, Debug, Serialize)]
pub struct CatalogHit {
    /// Endpoint.
    pub endpoint: Endpoint,
    /// Rank score (higher is better).
    pub score: i32,
}

/// In-memory catalog.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    endpoints: Vec<Endpoint>,
}

impl Catalog {
    /// Bundled seed.
    #[must_use]
    pub fn bundled() -> Self {
        Self::from_json(include_str!("../data/seed.json")).unwrap_or_default()
    }

    /// Parse a seed document `{ "endpoints": [ ... ] }`.
    ///
    /// # Errors
    ///
    /// JSON.
    pub fn from_json(src: &str) -> Result<Self, CatalogError> {
        let file: SeedFile =
            serde_json::from_str(src).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
        Ok(Self {
            endpoints: file.endpoints,
        })
    }

    /// Load every `*.yaml` / `*.yml` in `dir` (Treg-shaped `provider` + `endpoints`).
    ///
    /// # Errors
    ///
    /// IO or YAML.
    pub fn load_yaml_dir(dir: &Path) -> Result<Self, CatalogError> {
        let mut endpoints = Vec::new();
        let entries =
            std::fs::read_dir(dir).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if ext != "yaml" && ext != "yml" {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
            endpoints.extend(parse_treg_yaml(&text)?);
        }
        Ok(Self { endpoints })
    }

    /// All endpoints.
    #[must_use]
    pub fn all(&self) -> &[Endpoint] {
        &self.endpoints
    }

    /// Lookup by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|e| e.id == id)
    }

    /// Ranked job search. Empty query lists by id.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<CatalogHit> {
        let q = query.trim().to_ascii_lowercase();
        let mut hits: Vec<CatalogHit> = self
            .endpoints
            .iter()
            .filter_map(|ep| {
                let score = if q.is_empty() {
                    1
                } else {
                    score_endpoint(ep, &q)
                };
                (score > 0).then(|| CatalogHit {
                    endpoint: ep.clone(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.endpoint.id.cmp(&b.endpoint.id))
        });
        hits.truncate(limit.max(1));
        hits
    }
}

fn score_endpoint(ep: &Endpoint, q: &str) -> i32 {
    let mut score = 0;
    if ep.id.to_ascii_lowercase().contains(q) {
        score += 50;
    }
    if ep.capability.to_ascii_lowercase().contains(q) {
        score += 40;
    }
    if ep.provider.to_ascii_lowercase().contains(q) {
        score += 15;
    }
    if ep.name.to_ascii_lowercase().contains(q) {
        score += 20;
    }
    if ep.summary.to_ascii_lowercase().contains(q) {
        score += 10;
    }
    for job in &ep.jobs {
        let j = job.to_ascii_lowercase();
        if j == q {
            score += 80;
        } else if j.contains(q) || q.contains(&j) {
            score += 30;
        }
        for token in q.split_whitespace() {
            if j.contains(token) {
                score += 8;
            }
        }
    }
    score
}

#[derive(Deserialize)]
struct SeedFile {
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct TregYaml {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    endpoints: Vec<TregEndpoint>,
}

#[derive(Deserialize)]
struct TregEndpoint {
    id: String,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    cost: Option<TregCost>,
}

#[derive(Deserialize)]
struct TregCost {
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    currency: Option<String>,
}

fn parse_treg_yaml(text: &str) -> Result<Vec<Endpoint>, CatalogError> {
    let doc: TregYaml =
        serde_yaml::from_str(text).map_err(|e| CatalogError::InvalidDoc(e.to_string()))?;
    let provider = doc.provider.unwrap_or_else(|| "unknown".into());
    Ok(doc
        .endpoints
        .into_iter()
        .map(|e| {
            let cost_micro = e.cost.as_ref().and_then(treg_cost_to_micro);
            Endpoint {
                id: e.id.clone(),
                capability: e.capability.unwrap_or_else(|| e.id.clone()),
                provider: provider.clone(),
                name: e.name.unwrap_or_else(|| e.id.clone()),
                summary: e.summary.unwrap_or_default(),
                jobs: Vec::new(),
                method: e.method.unwrap_or_else(|| "GET".into()),
                path: e.path.unwrap_or_else(|| "/".into()),
                base_url: format!("mock://{provider}"),
                access: if cost_micro.is_some() {
                    Access::Platform
                } else {
                    Access::OwnKeyOnly
                },
                cost_micro,
                query: serde_json::Value::Null,
                routed_child: None,
            }
        })
        .collect())
}

fn treg_cost_to_micro(cost: &TregCost) -> Option<i64> {
    let value = cost.value?;
    if value <= 0.0 {
        return Some(0);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    match cost.currency.as_deref() {
        Some("usd" | "USD") => Some((value * 1_000_000.0) as i64),
        _ => Some((value * 10_000.0) as i64),
    }
}

/// Catalog / call errors.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// Unknown endpoint id.
    #[error("endpoint not found: {0}")]
    NotFound(String),
    /// Own key required; platform will not serve.
    #[error("conecte sua chave para {id} (provedor {provider})")]
    ConnectYourKey {
        /// Endpoint.
        id: String,
        /// Provider slug.
        provider: String,
    },
    /// Prepaid balance too low.
    #[error("saldo insuficiente")]
    PaymentRequired {
        /// Current balance.
        balance_micro: i64,
        /// What this call would cost.
        estimated_cost_micro: i64,
    },
    /// Bad document or argument.
    #[error("{0}")]
    Invalid(&'static str),
    /// Parse / IO.
    #[error("{0}")]
    InvalidDoc(String),
    /// Upstream HTTP.
    #[error("upstream: {0}")]
    Upstream(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::CallInput;

    #[test]
    fn search_finds_email_job() {
        let cat = Catalog::bundled();
        let hits = cat.search("encontrar e-mail", 8);
        assert!(
            hits.iter().any(|h| h.endpoint.id.contains("email.find")),
            "{hits:?}"
        );
    }

    #[test]
    fn yaml_ingest_reads_provider_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("demo.yaml"),
            "provider: demo\nendpoints:\n  - id: demo.ping\n    name: Ping\n    method: GET\n    path: /ping\n    cost: { value: 0, currency: usd }\n",
        )
        .unwrap();
        let cat = Catalog::load_yaml_dir(dir.path()).unwrap();
        assert_eq!(
            cat.get("demo.ping").map(|e| e.provider.as_str()),
            Some("demo")
        );
    }

    #[tokio::test]
    async fn ladder_own_key_is_free() {
        let svc = CatalogService::bundled();
        let paid = svc
            .call(CallInput {
                id: "hunter.people.email.find".into(),
                query: [
                    ("domain".into(), "stripe.com".into()),
                    ("full_name".into(), "Patrick Collison".into()),
                ]
                .into_iter()
                .collect(),
                subject: "local".into(),
                connection_secret: Some("sk_own".into()),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(paid.served_via, ServedVia::Connection);
        assert_eq!(paid.cost_micro, 0);
        let billed = svc
            .call(CallInput {
                id: "hunter.people.email.find".into(),
                query: [
                    ("domain".into(), "stripe.com".into()),
                    ("full_name".into(), "Patrick Collison".into()),
                ]
                .into_iter()
                .collect(),
                subject: "local".into(),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(billed.served_via, ServedVia::Platform);
        assert_eq!(billed.cost_micro, 10_000);
        assert!(svc.balance("local").unwrap() < SIGNUP_GRANT_MICRO);
    }

    #[tokio::test]
    async fn refuse_without_price_or_key() {
        let svc = CatalogService::bundled();
        let err = svc
            .call(CallInput {
                id: "internal.private.crm".into(),
                subject: "local".into(),
                ..CallInput::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, CatalogError::ConnectYourKey { .. }));
    }

    #[tokio::test]
    async fn payment_required_when_broke() {
        let svc = CatalogService::bundled();
        svc.balance("broke").unwrap();
        for _ in 0..200 {
            let _ = svc
                .call(CallInput {
                    id: "moz.backlinks.lookup".into(),
                    query: std::iter::once(("domain".into(), "example.com".into())).collect(),
                    subject: "broke".into(),
                    ..CallInput::default()
                })
                .await;
        }
        let err = svc
            .call(CallInput {
                id: "moz.backlinks.lookup".into(),
                query: std::iter::once(("domain".into(), "example.com".into())).collect(),
                subject: "broke".into(),
                ..CallInput::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, CatalogError::PaymentRequired { .. }));
    }

    #[tokio::test]
    async fn routed_names_child() {
        let svc = CatalogService::bundled();
        let out = svc
            .call(CallInput {
                id: "treg.people.email.find".into(),
                query: [
                    ("domain".into(), "stripe.com".into()),
                    ("full_name".into(), "Patrick".into()),
                ]
                .into_iter()
                .collect(),
                subject: "local".into(),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(out.child.as_deref(), Some("hunter.people.email.find"));
        assert_eq!(out.served_via, ServedVia::Routed);
    }
}
