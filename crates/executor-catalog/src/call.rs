//! Credential ladder and faithful call runtime.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::money::MemoryLedger;
use crate::{Access, Catalog, CatalogError, Endpoint};

/// Team-owned relay tool (secret never listed).
#[derive(Clone, Debug, Serialize)]
pub struct TeamTool {
    /// Id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Provider slug this tool covers.
    pub provider: String,
    /// Upstream origin.
    pub base_url: String,
}

#[derive(Clone, Debug)]
struct TeamToolStored {
    meta: TeamTool,
    secret: String,
}

/// How a call was authorized (disclosed on the response).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServedVia {
    /// Team-registered tool + binding.
    TeamTool,
    /// Saved Executor connection (own key).
    Connection,
    /// Verified public / anonymous route.
    Anonymous,
    /// Platform (treg-style) key, metered.
    Platform,
    /// Routed capability that named a child.
    Routed,
}

impl ServedVia {
    /// Own-key rungs are never metered.
    #[must_use]
    pub const fn metered(self) -> bool {
        matches!(self, Self::Platform | Self::Routed)
    }
}

/// Inputs for one `/call`.
#[derive(Clone, Debug, Default)]
pub struct CallInput {
    /// Catalog endpoint id.
    pub id: String,
    /// Query string fields.
    pub query: BTreeMap<String, String>,
    /// JSON body (POST).
    pub body: Option<Value>,
    /// Subject for the ledger.
    pub subject: String,
    /// Own-key secret when a connection exists for this provider.
    pub connection_secret: Option<String>,
    /// Team-tool secret when a team tool matches this provider.
    pub team_tool_secret: Option<String>,
}

/// Result of a catalog call (never includes secrets).
#[derive(Clone, Debug, Serialize)]
pub struct CallOutcome {
    /// HTTP-ish status from upstream or mock.
    pub status: u16,
    /// Ladder rung that served.
    pub served_via: ServedVia,
    /// Charged micro-USD (0 if own key / anonymous / free).
    pub cost_micro: i64,
    /// Child endpoint when routed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
    /// Upstream / mock body.
    pub body: Value,
}

/// Catalog + ledger + call counter.
pub struct CatalogService {
    catalog: Catalog,
    ledger: MemoryLedger,
    calls: AtomicU64,
    team_tools: Mutex<HashMap<String, TeamToolStored>>,
}

impl CatalogService {
    /// Bundled seed catalog.
    #[must_use]
    pub fn bundled() -> Self {
        Self {
            catalog: Catalog::bundled(),
            ledger: MemoryLedger::new(),
            calls: AtomicU64::new(1),
            team_tools: Mutex::new(HashMap::new()),
        }
    }

    /// From an already-parsed catalog (tests / YAML ingest).
    #[must_use]
    pub fn from_catalog(catalog: Catalog) -> Self {
        Self {
            catalog,
            ledger: MemoryLedger::new(),
            calls: AtomicU64::new(1),
            team_tools: Mutex::new(HashMap::new()),
        }
    }

    /// Bundled seed plus YAML from `EXECUTOR_CATALOG_DIR` and `{data_dir}/catalog`.
    #[must_use]
    pub fn from_env_and_data_dir(data_dir: Option<&std::path::Path>) -> Self {
        Self::from_catalog(Catalog::open_dirs(&crate::yaml::catalog_search_dirs(
            data_dir,
        )))
    }

    /// Searchable catalog.
    #[must_use]
    pub const fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Ledger.
    #[must_use]
    pub const fn ledger(&self) -> &MemoryLedger {
        &self.ledger
    }

    /// Balance including one-time signup grant.
    ///
    /// # Errors
    ///
    /// Ledger grant failures.
    pub fn balance(&self, subject: &str) -> Result<i64, CatalogError> {
        self.ledger.ensure_signup_grant(subject)
    }

    /// Mock top-up.
    ///
    /// # Errors
    ///
    /// Non-positive amount or ledger failure.
    pub fn topup(&self, subject: &str, micro: i64) -> Result<i64, CatalogError> {
        self.ledger.ensure_signup_grant(subject)?;
        self.ledger.grant(subject, micro)
    }

    /// Register a team tool. Secret is write-only.
    pub fn register_team_tool(
        &self,
        name: String,
        provider: String,
        base_url: String,
        secret: String,
    ) -> TeamTool {
        let id = format!("tt_{}", self.calls.fetch_add(1, Ordering::Relaxed));
        let meta = TeamTool {
            id: id.clone(),
            name,
            provider,
            base_url,
        };
        self.team_tools.lock().insert(
            id,
            TeamToolStored {
                meta: meta.clone(),
                secret,
            },
        );
        meta
    }

    /// Metadata only.
    #[must_use]
    pub fn list_team_tools(&self) -> Vec<TeamTool> {
        self.team_tools
            .lock()
            .values()
            .map(|t| t.meta.clone())
            .collect()
    }

    /// Secret for the first team tool covering `provider`.
    #[must_use]
    pub fn team_secret_for(&self, provider: &str) -> Option<String> {
        self.team_tools
            .lock()
            .values()
            .find(|t| t.meta.provider == provider)
            .map(|t| t.secret.clone())
    }

    /// Run the credential ladder and invoke.
    ///
    /// # Errors
    ///
    /// Missing endpoint, refuse-without-price, 402, upstream.
    pub async fn call(&self, input: CallInput) -> Result<CallOutcome, CatalogError> {
        let endpoint = self
            .catalog
            .get(&input.id)
            .ok_or_else(|| CatalogError::NotFound(input.id.clone()))?
            .clone();
        if endpoint.routed_child.is_none() {
            enforce_strict_query(&endpoint, &input)?;
        }
        if let Some(child_id) = endpoint.routed_child.as_ref() {
            let mut child_input = input.clone();
            child_input.id = child_id.clone();
            let mut child = Box::pin(self.call(child_input)).await?;
            child.child = Some(child_id.clone());
            child.served_via = ServedVia::Routed;
            return Ok(child);
        }
        let mut input = input;
        if input.team_tool_secret.is_none() {
            input.team_tool_secret = self.team_secret_for(&endpoint.provider);
        }
        let (via, secret) = ladder(&endpoint, &input)?;
        let call_id = format!("call_{}", self.calls.fetch_add(1, Ordering::Relaxed));
        let meter = if via.metered() {
            endpoint.cost_micro.unwrap_or(0)
        } else {
            0
        };
        if meter > 0 {
            self.ledger.ensure_signup_grant(&input.subject)?;
            self.ledger.reserve(&input.subject, &call_id, meter)?;
        }
        match invoke_endpoint(&endpoint, &input, secret.as_deref()).await {
            Ok((status, body)) => {
                if meter > 0 {
                    self.ledger.settle(&call_id, meter)?;
                }
                Ok(CallOutcome {
                    status,
                    served_via: via,
                    cost_micro: meter,
                    child: None,
                    body,
                })
            }
            Err(err) => {
                if meter > 0 {
                    self.ledger.release(&call_id)?;
                }
                Err(err)
            }
        }
    }
}

fn ladder(
    endpoint: &Endpoint,
    input: &CallInput,
) -> Result<(ServedVia, Option<String>), CatalogError> {
    if let Some(secret) = input.team_tool_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok((ServedVia::TeamTool, Some(secret.clone())));
    }
    if let Some(secret) = input.connection_secret.as_ref().filter(|s| !s.is_empty()) {
        return Ok((ServedVia::Connection, Some(secret.clone())));
    }
    match endpoint.access {
        Access::Anonymous => Ok((ServedVia::Anonymous, None)),
        Access::Platform if endpoint.cost_micro.is_some() => Ok((ServedVia::Platform, None)),
        Access::Platform | Access::OwnKeyOnly => Err(CatalogError::ConnectYourKey {
            id: endpoint.id.clone(),
            provider: endpoint.provider.clone(),
        }),
    }
}

fn enforce_strict_query(endpoint: &Endpoint, input: &CallInput) -> Result<(), CatalogError> {
    if !endpoint.strict_query {
        return Ok(());
    }
    let Some(fields) = endpoint.query.as_object() else {
        return Err(CatalogError::ParameterInvalid(
            "use só os parâmetros de query declarados; omita o corpo".into(),
        ));
    };
    if input.body.is_some() {
        return Err(CatalogError::ParameterInvalid(
            "use só os parâmetros de query declarados, uma vez cada; inclua os obrigatórios e omita o corpo".into(),
        ));
    }
    for key in input.query.keys() {
        let Some(spec) = fields.get(key) else {
            return Err(CatalogError::ParameterInvalid(format!(
                "parâmetro não declarado: {key}"
            )));
        };
        if let Some(allowed) = spec.get("enum").and_then(Value::as_array) {
            let value = input.query.get(key).map_or("", String::as_str);
            let ok = allowed.iter().any(|item| item.as_str() == Some(value));
            if !ok {
                return Err(CatalogError::ParameterInvalid(format!(
                    "valor não permitido para {key}"
                )));
            }
        }
    }
    for (name, spec) in fields {
        if spec.get("required").and_then(Value::as_bool) == Some(true) {
            let empty = input
                .query
                .get(name)
                .is_none_or(|value| value.trim().is_empty());
            if empty {
                return Err(CatalogError::ParameterInvalid(format!(
                    "parâmetro obrigatório ausente: {name}"
                )));
            }
        }
    }
    Ok(())
}

async fn invoke_endpoint(
    endpoint: &Endpoint,
    input: &CallInput,
    secret: Option<&str>,
) -> Result<(u16, Value), CatalogError> {
    let base = resolve_base(&endpoint.base_url);
    if base.starts_with("mock://") || base.is_empty() {
        return Ok((200, mock_body(endpoint, input, secret)));
    }
    let mut url = format!("{}{}", base.trim_end_matches('/'), endpoint.path);
    if endpoint.method.eq_ignore_ascii_case("GET") && !input.query.is_empty() {
        let qs: Vec<String> = input
            .query
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding_lite(k), urlencoding_lite(v)))
            .collect();
        url.push('?');
        url.push_str(&qs.join("&"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| CatalogError::Upstream(e.to_string()))?;
    let method =
        reqwest::Method::from_bytes(endpoint.method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut req = client.request(method.clone(), &url);
    if let Some(token) = secret {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if method != reqwest::Method::GET
        && method != reqwest::Method::HEAD
        && let Some(body) = &input.body
    {
        req = req.json(body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| CatalogError::Upstream(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .json::<Value>()
        .await
        .unwrap_or_else(|_| json!({"ok": status < 400}));
    Ok((status, body))
}

fn resolve_base(template: &str) -> String {
    if let Some(name) = template.strip_prefix('$') {
        return std::env::var(name).unwrap_or_default();
    }
    template.to_owned()
}

fn mock_body(endpoint: &Endpoint, input: &CallInput, secret: Option<&str>) -> Value {
    let used_own_key = secret.is_some();
    match endpoint.id.as_str() {
        "demo.echo" => json!({
            "ok": true,
            "text": input.query.get("text").cloned().unwrap_or_else(|| "olá".into()),
        }),
        "hunter.people.email.find" => {
            let domain = input
                .query
                .get("domain")
                .cloned()
                .unwrap_or_else(|| "example.com".into());
            let name = input
                .query
                .get("full_name")
                .cloned()
                .unwrap_or_else(|| "Ada Lovelace".into());
            let local = name
                .split_whitespace()
                .next()
                .unwrap_or("ada")
                .to_ascii_lowercase();
            json!({
                "data": { "email": format!("{local}@{domain}"), "score": 92 },
                "own_key": used_own_key,
            })
        }
        "hunter.people.email.verify" => json!({
            "data": {
                "email": input.query.get("email"),
                "status": "valid",
            }
        }),
        "github.user.get" => json!({"login": "octocat", "id": 1, "mock": true}),
        "moz.backlinks.lookup" => json!({
            "domain": input.query.get("domain"),
            "backlinks": 42,
        }),
        "internal.private.crm" => json!({
            "contacts": [{"name": "Ada", "email": "ada@example.com"}],
            "own_key": true,
        }),
        other => json!({"id": other, "ok": true, "own_key": used_own_key}),
    }
}

fn urlencoding_lite(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                for b in c.encode_utf8(&mut [0; 4]).as_bytes() {
                    out.push('%');
                    let hex = b"0123456789ABCDEF";
                    out.push(char::from(hex[usize::from(b >> 4)]));
                    out.push(char::from(hex[usize::from(b & 0x0f)]));
                }
            }
        }
    }
    out
}
