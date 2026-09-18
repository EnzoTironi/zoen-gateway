//! Treg-style endpoint catalog: search by job, prices, credential ladder.
//!
//! Does not vendor the Python tree. Seed JSON ships in-tree; YAML ingest reads
//! Treg-shaped documents when the operator points at a directory.

#![allow(clippy::module_name_repetitions)]

mod call;
mod money;
mod yaml;

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use call::{
    ArenaCapability, ArenaEndpoint, CallInput, CallOutcome, CatalogService, ServedVia, TeamTool,
    TopupIntent, cli_secret_env,
};
pub use money::{MemoryLedger, SIGNUP_GRANT_MICRO};
pub use yaml::catalog_search_dirs;

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
    /// When true, `/call` rejects undeclared/missing query params and bodies (Treg `strict_query`).
    #[serde(default)]
    pub strict_query: bool,
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
        Ok(Self {
            endpoints: yaml::load_yaml_dir(dir)?,
        })
    }

    /// Bundled seed plus YAML dirs (`EXECUTOR_CATALOG_DIR`, optional `{data_dir}/catalog`).
    #[must_use]
    pub fn open_dirs(dirs: &[std::path::PathBuf]) -> Self {
        let mut catalog = Self::bundled();
        for dir in dirs {
            match Self::load_yaml_dir(dir) {
                Ok(extra) => catalog.merge(extra),
                Err(err) => {
                    tracing::warn!(path = %dir.display(), error = %err, "catalog yaml dir skipped");
                }
            }
        }
        catalog
    }

    /// Insert endpoints whose ids are not already present (seed wins).
    pub fn merge(&mut self, other: Self) {
        for endpoint in other.endpoints {
            if !self.endpoints.iter().any(|e| e.id == endpoint.id) {
                self.endpoints.push(endpoint);
            }
        }
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

    /// Unique providers with endpoint counts (sorted by slug).
    #[must_use]
    pub fn providers(&self) -> Vec<(String, usize)> {
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for endpoint in &self.endpoints {
            *counts.entry(endpoint.provider.clone()).or_insert(0) += 1;
        }
        counts.into_iter().collect()
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
    /// Strict query / undeclared parameter.
    #[error("{0}")]
    ParameterInvalid(String),
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
        assert_eq!(cat.get("demo.ping").and_then(|e| e.cost_micro), Some(0));
    }

    #[test]
    fn yaml_strict_query_and_usd_price() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("harvestapi.yaml"),
            r"provider: harvestapi
endpoints:
  - id: harvestapi.linkedin.user.profile
    name: Perfil LinkedIn
    method: GET
    path: /linkedin/profile
    capability: linkedin.user.profile
    strict_query: true
    input:
      queryParams:
        publicIdentifier: { type: string, required: true, example: williamhgates }
        url: { type: string, required: false }
    cost: { value: 0.0064, currency: USD }
",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("adapters.yaml"),
            "provider: adapters\nendpoints: []\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("hunter.extended.yaml"),
            "provider: hunter\nendpoints:\n  - id: hunter.x.should-skip\n    name: skip\n",
        )
        .unwrap();
        let cat = Catalog::load_yaml_dir(dir.path()).unwrap();
        let ep = cat.get("harvestapi.linkedin.user.profile").unwrap();
        assert!(ep.strict_query);
        assert_eq!(ep.cost_micro, Some(6400));
        assert!(cat.get("hunter.x.should-skip").is_none());
    }

    #[test]
    fn seed_wins_on_yaml_merge() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hunter.yaml"),
            "provider: hunter\nendpoints:\n  - id: hunter.people.email.find\n    name: YAML\n    cost: { value: 1, currency: credit }\n  - id: hunter.account.get\n    name: Account\n    method: GET\n    path: /account\n",
        )
        .unwrap();
        let mut cat = Catalog::bundled();
        cat.merge(Catalog::load_yaml_dir(dir.path()).unwrap());
        let find = cat.get("hunter.people.email.find").unwrap();
        assert_eq!(find.cost_micro, Some(10_000));
        assert_eq!(find.name, "Encontrar e-mail profissional");
        assert!(cat.get("hunter.account.get").is_some());
    }

    #[tokio::test]
    async fn strict_query_rejects_undeclared() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("strict.yaml"),
            r"provider: harvestapi
endpoints:
  - id: harvestapi.linkedin.user.profile
    method: GET
    path: /linkedin/profile
    strict_query: true
    input:
      queryParams:
        publicIdentifier: { type: string, required: true }
    cost: { value: 0.01, currency: USD }
",
        )
        .unwrap();
        let svc = CatalogService::from_catalog(Catalog::load_yaml_dir(dir.path()).unwrap());
        let err = svc
            .call(CallInput {
                id: "harvestapi.linkedin.user.profile".into(),
                query: std::iter::once(("main".into(), "1".into())).collect(),
                subject: "local".into(),
                ..CallInput::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, CatalogError::ParameterInvalid(_)), "{err:?}");
        let ok = svc
            .call(CallInput {
                id: "harvestapi.linkedin.user.profile".into(),
                query: std::iter::once(("publicIdentifier".into(), "williamhgates".into()))
                    .collect(),
                subject: "local".into(),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(ok.status, 200);
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

    #[tokio::test]
    async fn overflow_after_platform_500() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer ov-secret",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "via": "overflow"
                })),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_json(serde_json::json!({
                    "error": "platform_out"
                })),
            )
            .mount(&server)
            .await;
        let mut catalog = Catalog::bundled();
        catalog.merge(Catalog {
            endpoints: vec![Endpoint {
                id: "moz.overflow.probe".into(),
                capability: "backlinks.lookup".into(),
                provider: "moz".into(),
                name: "Overflow probe".into(),
                summary: "wiremock".into(),
                jobs: vec![],
                method: "GET".into(),
                path: "/probe".into(),
                base_url: server.uri(),
                access: Access::Platform,
                cost_micro: Some(1_000),
                query: serde_json::json!({}),
                routed_child: None,
                strict_query: false,
            }],
        });
        let svc = CatalogService::from_catalog(catalog);
        svc.register_overflow_relay(
            "moz-overflow".into(),
            "moz".into(),
            server.uri(),
            "ov-secret".into(),
        );
        let out = svc
            .call(CallInput {
                id: "moz.overflow.probe".into(),
                subject: "overflow".into(),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(out.served_via, ServedVia::Overflow);
        assert_eq!(out.body["via"], "overflow");
        svc.set_overflow_opt_out(true);
        let skipped = svc
            .call(CallInput {
                id: "moz.overflow.probe".into(),
                subject: "overflow".into(),
                ..CallInput::default()
            })
            .await
            .unwrap();
        assert_eq!(skipped.status, 500);
        assert_eq!(skipped.served_via, ServedVia::Platform);
    }

    #[test]
    fn arena_lists_email_competitors() {
        let caps = CatalogService::bundled().arena_capabilities();
        let email = caps.iter().find(|c| c.capability == "people.email.find");
        assert!(email.is_some_and(|c| c.endpoints.len() >= 2), "{caps:?}");
    }

    #[test]
    fn jail_denies_shells_and_allows_vendor() {
        let svc = CatalogService::bundled();
        assert!(svc.cli_allowed("stripe"));
        assert!(!svc.cli_allowed("bash"));
        assert!(svc.register_cli("echo").is_ok());
        assert!(svc.cli_allowed("echo"));
        assert!(svc.register_cli("../bash").is_err());
    }

    #[test]
    fn topup_intent_credits_only_on_confirm() {
        let svc = CatalogService::bundled();
        let before = svc.balance("pay").unwrap();
        let intent = svc.create_topup("pay", 2_000_000).unwrap();
        assert_eq!(svc.ledger().balance("pay"), before);
        let after = svc.confirm_topup(&intent.id).unwrap();
        assert_eq!(after, before + 2_000_000);
    }
}
