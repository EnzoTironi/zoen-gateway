//! Remaining agent-facing core-tools: detect, OAuth clients, providers, toolkits, policy update.

use executor_core::{
    ExecutorError, IntegrationSlug, KV_OAUTH_CLIENTS, KV_OAUTH_SESSIONS, KV_TOOLKITS, Owner,
    PolicyId, PolicyPattern, ToolResult, Toolkit, unix_now_ms,
};
use serde_json::{Value, json};

use crate::Inner;
use crate::oauth::{authorization_request, pkce_pair, register_client};

impl Inner {
    pub(crate) fn st_integrations_detect(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let url = req_str(args, "url")?;
        let mut results: Vec<Value> = self
            .plugins
            .values()
            .filter_map(|p| p.detect(url))
            .map(|d| {
                json!({
                    "kind": d.kind.as_str(),
                    "confidence": d.confidence,
                    "endpoint": d.endpoint,
                    "name": d.name,
                    "slug": d.slug,
                })
            })
            .collect();
        results.sort_by(|a, b| {
            confidence_rank(a.get("confidence"))
                .cmp(&confidence_rank(b.get("confidence")))
                .then_with(|| {
                    a.get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .cmp(b.get("kind").and_then(Value::as_str).unwrap_or(""))
                })
        });
        Ok(ToolResult::ok(json!({ "results": results })))
    }

    pub(crate) fn st_connections_create_handoff(args: &Value) -> Result<ToolResult, ExecutorError> {
        let integration = req_str(args, "integration")?;
        let owner = args.get("owner").and_then(Value::as_str).unwrap_or("org");
        Ok(ToolResult::ok(json!({
            "url": format!(
                "http://127.0.0.1:4788/?handoff=connection&integration={integration}&owner={owner}"
            ),
            "instructions": "This daemon has no web UI. Ask the user to supply a secret ref, then call executor.coreTools.connections.create. Do not paste client secrets into chat.",
        })))
    }

    #[allow(clippy::unnecessary_wraps)] // same Result shape as sibling core-tools
    pub(crate) fn st_providers_list(&self) -> Result<ToolResult, ExecutorError> {
        Ok(ToolResult::ok(json!({
            "providers": self.secrets.list_providers(),
        })))
    }

    pub(crate) fn st_providers_items(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let provider = req_str(args, "provider")?;
        let items: Vec<Value> = self
            .secrets
            .list_items(provider)?
            .into_iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect();
        Ok(ToolResult::ok(json!({ "items": items })))
    }

    pub(crate) fn st_policies_update(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let id = PolicyId::new(req_str(args, "id")?)?;
        let mut rows = self.store.list_policies()?;
        let Some(row) = rows.iter_mut().find(|p| p.id == id) else {
            return Err(ExecutorError::InvalidArgs(format!("policy {id} not found")));
        };
        if let Some(pattern) = args.get("pattern").and_then(Value::as_str) {
            row.pattern = PolicyPattern::new(pattern).map_err(ExecutorError::InvalidPattern)?;
        }
        if let Some(action) = args.get("action").and_then(Value::as_str) {
            row.action = parse_policy_action(action)?;
        }
        let updated = row.clone();
        self.store.put_policy(updated.clone())?;
        Ok(ToolResult::ok(json!({
            "id": updated.id.as_str(),
            "owner": updated.owner.as_str(),
            "pattern": updated.pattern.as_str(),
            "action": updated.action,
            "position": updated.position,
        })))
    }

    pub(crate) fn st_oauth_clients_list(&self) -> Result<ToolResult, ExecutorError> {
        let clients = self.store.list_kv(KV_OAUTH_CLIENTS)?;
        Ok(ToolResult::ok(json!({ "clients": clients })))
    }

    pub(crate) fn st_oauth_clients_create(
        &self,
        args: &Value,
    ) -> Result<ToolResult, ExecutorError> {
        if args.get("clientSecret").is_some() {
            return Err(ExecutorError::InvalidArgs(
                "oauth.clients.create is public-client only; use oauth.clients.createHandoff for confidential apps".into(),
            ));
        }
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let slug = req_str(args, "slug")?;
        let id = format!("{}:{slug}", owner.as_str());
        let body = json!({
            "owner": owner.as_str(),
            "slug": slug,
            "grant": args.get("grant").and_then(Value::as_str).unwrap_or("authorization_code"),
            "authorizationUrl": req_str(args, "authorizationUrl")?,
            "tokenUrl": req_str(args, "tokenUrl")?,
            "resource": args.get("resource"),
            "clientId": req_str(args, "clientId")?,
            "origin": { "kind": "manual" },
            "originIntegration": args.get("originIntegration"),
        });
        self.store.put_kv(KV_OAUTH_CLIENTS, &id, body.clone())?;
        Ok(ToolResult::ok(body))
    }

    pub(crate) fn st_oauth_clients_create_handoff(
        args: &Value,
    ) -> Result<ToolResult, ExecutorError> {
        let integration = req_str(args, "integration")?;
        Ok(ToolResult::ok(json!({
            "url": format!(
                "http://127.0.0.1:4788/?handoff=oauth-client&integration={integration}"
            ),
            "instructions": "Ask the user to register the confidential OAuth app outside this agent. After they finish, call oauth.clients.list, then oauth.start. Do not collect the client secret in chat.",
        })))
    }

    pub(crate) async fn st_oauth_clients_register_dynamic(
        &self,
        args: &Value,
    ) -> Result<ToolResult, ExecutorError> {
        let endpoint = req_str(args, "registrationEndpoint")?;
        let redirect = args
            .get("redirectUri")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:4788/api/oauth/callback");
        let client = register_client(endpoint, redirect, self.limits.http_timeout).await?;
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let slug = req_str(args, "slug")?;
        let id = format!("{}:{slug}", owner.as_str());
        let body = json!({
            "owner": owner.as_str(),
            "slug": slug,
            "grant": "authorization_code",
            "authorizationUrl": req_str(args, "authorizationUrl")?,
            "tokenUrl": req_str(args, "tokenUrl")?,
            "resource": args.get("resource"),
            "clientId": client.client_id,
            "origin": {
                "kind": "dynamic_client_registration",
                "integration": args.get("originIntegration"),
            },
        });
        self.store.put_kv(KV_OAUTH_CLIENTS, &id, body)?;
        Ok(ToolResult::ok(json!({ "client": id })))
    }

    pub(crate) fn st_oauth_clients_remove(
        &self,
        args: &Value,
    ) -> Result<ToolResult, ExecutorError> {
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let slug = req_str(args, "slug")?;
        let id = format!("{}:{slug}", owner.as_str());
        let existed = self.store.delete_kv(KV_OAUTH_CLIENTS, &id)?;
        Ok(ToolResult::ok(json!({ "removed": existed })))
    }

    pub(crate) async fn st_oauth_probe(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let url = req_str(args, "url")?;
        let metadata = fetch_as_metadata(url, self.limits.http_timeout).await?;
        Ok(ToolResult::ok(metadata))
    }

    pub(crate) fn st_oauth_start(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let client_slug = req_str(args, "client")?;
        let client_owner = args
            .get("clientOwner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let id = format!("{}:{client_slug}", client_owner.as_str());
        let Some(client) = self.store.get_kv(KV_OAUTH_CLIENTS, &id)? else {
            return Err(ExecutorError::InvalidArgs(format!(
                "oauth client {id} not found"
            )));
        };
        let auth_url = client
            .get("authorizationUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutorError::InvalidArgs("client missing authorizationUrl".into()))?;
        let client_id = client
            .get("clientId")
            .and_then(Value::as_str)
            .ok_or_else(|| ExecutorError::InvalidArgs("client missing clientId".into()))?;
        let redirect = args
            .get("redirectUri")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:4788/api/oauth/callback");
        let (verifier, challenge) = pkce_pair();
        let state = format!("st_{:x}", unix_now_ms());
        let url = authorization_request(auth_url, client_id, redirect, &[], &state, &challenge);
        self.store.put_kv(
            KV_OAUTH_SESSIONS,
            &state,
            json!({
                "state": state,
                "verifier": verifier,
                "client": id,
                "owner": args.get("owner").and_then(Value::as_str).unwrap_or("org"),
                "name": args.get("name"),
                "integration": args.get("integration"),
            }),
        )?;
        Ok(ToolResult::ok(json!({
            "status": "redirect",
            "authorizationUrl": url,
            "state": state,
        })))
    }

    pub(crate) fn st_oauth_cancel(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let state = req_str(args, "state")?;
        let existed = self.store.delete_kv(KV_OAUTH_SESSIONS, state)?;
        Ok(ToolResult::ok(json!({ "cancelled": existed })))
    }

    pub(crate) fn st_toolkits_list(&self) -> Result<ToolResult, ExecutorError> {
        Ok(ToolResult::ok(json!({
            "toolkits": self.store.list_kv(KV_TOOLKITS)?,
        })))
    }

    pub(crate) fn st_toolkits_create(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let slug = req_str(args, "slug")?;
        IntegrationSlug::new(slug)?;
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let toolkit = Toolkit {
            id: args
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("tk_{slug}"), ToOwned::to_owned),
            owner,
            slug: slug.to_owned(),
            name: args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(slug)
                .to_owned(),
            connections: args
                .get("connections")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            policies: Vec::new(),
        };
        let body = serde_json::to_value(&toolkit)
            .map_err(|e| ExecutorError::InvalidArgs(e.to_string()))?;
        self.store.put_kv(KV_TOOLKITS, slug, body.clone())?;
        Ok(ToolResult::ok(body))
    }

    pub(crate) fn st_toolkits_remove(&self, args: &Value) -> Result<ToolResult, ExecutorError> {
        let slug = req_str(args, "slug")?;
        let existed = self.store.delete_kv(KV_TOOLKITS, slug)?;
        Ok(ToolResult::ok(json!({ "removed": existed })))
    }
}

fn parse_policy_action(raw: &str) -> Result<executor_core::PolicyAction, ExecutorError> {
    match raw {
        "approve" => Ok(executor_core::PolicyAction::Approve),
        "require_approval" => Ok(executor_core::PolicyAction::RequireApproval),
        "block" => Ok(executor_core::PolicyAction::Block),
        other => Err(ExecutorError::InvalidArgs(format!(
            "unknown action {other}"
        ))),
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ExecutorError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ExecutorError::InvalidArgs(format!("missing {key}")))
}

fn confidence_rank(value: Option<&Value>) -> u8 {
    match value.and_then(Value::as_str).unwrap_or("") {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 9,
    }
}

async fn fetch_as_metadata(
    url: &str,
    timeout: std::time::Duration,
) -> Result<Value, ExecutorError> {
    let client = crate::oauth::http();
    let candidates = [
        url.to_owned(),
        format!(
            "{}/.well-known/oauth-authorization-server",
            url.trim_end_matches('/')
        ),
        format!(
            "{}/.well-known/openid-configuration",
            url.trim_end_matches('/')
        ),
    ];
    let mut last = ExecutorError::Plugin("oauth probe: no metadata".into());
    for candidate in candidates {
        match client.get(&candidate).timeout(timeout).send().await {
            Ok(response) if response.status().is_success() => {
                let body: Value = response
                    .json()
                    .await
                    .map_err(|e| ExecutorError::Plugin(format!("oauth probe body: {e}")))?;
                if body.get("authorization_endpoint").is_some()
                    || body.get("token_endpoint").is_some()
                {
                    return Ok(json!({
                        "issuer": body.get("issuer"),
                        "authorizationUrl": body.get("authorization_endpoint"),
                        "tokenUrl": body.get("token_endpoint"),
                        "resource": body.get("resource"),
                        "scopesSupported": body.get("scopes_supported"),
                        "registrationEndpoint": body.get("registration_endpoint"),
                        "tokenEndpointAuthMethodsSupported": body.get("token_endpoint_auth_methods_supported"),
                        "clientIdMetadataDocumentSupported": body.get("client_id_metadata_document_supported"),
                    }));
                }
            }
            Ok(_) => {
                last = ExecutorError::Plugin(format!("oauth probe HTTP error at {candidate}"));
            }
            Err(err) => {
                last = ExecutorError::Plugin(format!("oauth probe: {err}"));
            }
        }
    }
    Err(last)
}
