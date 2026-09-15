//! GraphQL integration plugin.

#![allow(clippy::module_name_repetitions)]

mod extract;

use std::time::Duration;

use async_trait::async_trait;
use executor_core::{
    AuthKind, AuthMethod, Detection, DetectionConfidence, HealthCheckCtx, HealthVerdict,
    IntegrationConfig, IntegrationPlugin, InvokeCtx, PluginError, PluginId, ResolveToolsCtx,
    ResolvedTools, ToolError, ToolResult, tool_error_from_http,
};
use reqwest::Client;
use serde_json::{Value, json};
use tracing::instrument;

pub use extract::tools_from_introspection;

/// First-party GraphQL plugin.
pub struct GraphqlPlugin {
    client: Client,
}

impl GraphqlPlugin {
    /// Pooled HTTP client.
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(32)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }
}

impl Default for GraphqlPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IntegrationPlugin for GraphqlPlugin {
    fn id(&self) -> PluginId {
        PluginId::graphql()
    }

    fn detect(&self, candidate: &str) -> Option<Detection> {
        let lower = candidate.to_ascii_lowercase();
        if lower.contains("graphql") {
            Some(Detection {
                kind: PluginId::graphql(),
                confidence: DetectionConfidence::High,
                endpoint: candidate.to_owned(),
                name: "GraphQL".into(),
                slug: "graphql".into(),
            })
        } else {
            None
        }
    }

    fn describe_auth(&self, config: &IntegrationConfig) -> Vec<AuthMethod> {
        if let Some(arr) = config
            .get("authenticationTemplate")
            .and_then(Value::as_array)
        {
            let parsed: Vec<AuthMethod> = arr
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            if !parsed.is_empty() {
                return parsed;
            }
        }
        let mut methods = vec![AuthMethod::none(), AuthMethod::bearer()];
        if let (Some(auth), Some(token)) = (
            config.get("authorizationUrl").and_then(Value::as_str),
            config.get("tokenUrl").and_then(Value::as_str),
        ) {
            let scopes = config
                .get("scopes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            methods.push(AuthMethod::oauth(auth, token, scopes));
        }
        methods
    }

    #[instrument(skip(self, ctx))]
    async fn resolve_tools(&self, ctx: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError> {
        let schema = if let Some(s) = ctx.config.get("schema") {
            s.clone()
        } else {
            introspect(&self.client, ctx.config, ctx.values, ctx.timeout).await?
        };
        let inner = schema.get("__schema").cloned().unwrap_or(schema);
        ensure_spec_budget(&inner, ctx.max_spec_bytes)?;
        let tools = tools_from_introspection(&inner)?;
        if tools.len() > ctx.max_tools {
            return Err(PluginError::new(format!(
                "schema produced {} fields (max {})",
                tools.len(),
                ctx.max_tools
            )));
        }
        Ok(ResolvedTools {
            tools,
            definitions: None,
            incomplete: false,
            incomplete_reason: None,
        })
    }

    #[instrument(skip(self, ctx))]
    async fn invoke(&self, ctx: InvokeCtx<'_>) -> Result<ToolResult, PluginError> {
        let endpoint = ctx
            .config_endpoint()
            .ok_or_else(|| PluginError::new("missing GraphQL endpoint"))?;
        let meta = ctx
            .tool
            .plugin_meta
            .as_ref()
            .ok_or_else(|| PluginError::new("missing plugin_meta"))?;
        let query = meta
            .get("operationString")
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError::new("missing operationString"))?;
        let mut req = self
            .client
            .post(&endpoint)
            .timeout(ctx.timeout)
            .json(&json!({
                "query": query,
                "variables": ctx.args,
            }));
        req = apply_auth(req, &ctx);
        let response = req
            .send()
            .await
            .map_err(|e| PluginError::new(format!("graphql: {e}")))?;
        let status = response.status();
        let header_pairs: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| PluginError::new(format!("graphql body: {e}")))?;
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
        if !status.is_success() {
            return Ok(ToolResult::fail(tool_error_from_http(
                status.as_u16(),
                &header_pairs,
                &body,
            )));
        }
        if let Some(errors) = body.get("errors") {
            return Ok(ToolResult::fail(ToolError {
                code: "graphql_error".into(),
                message: errors
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("GraphQL error")
                    .to_owned(),
                status: None,
                details: Some(errors.clone()),
                retryable: Some(false),
            }));
        }
        Ok(ToolResult::ok(
            body.get("data").cloned().unwrap_or(Value::Null),
        ))
    }

    async fn check_health(&self, _ctx: HealthCheckCtx<'_>) -> Result<HealthVerdict, PluginError> {
        Ok(HealthVerdict::Unknown)
    }
}

fn ensure_spec_budget(schema: &Value, max: usize) -> Result<(), PluginError> {
    let bytes = serde_json::to_vec(schema).unwrap_or_default().len();
    if bytes > max {
        return Err(PluginError::new(format!(
            "schema is {bytes} bytes (max {max})"
        )));
    }
    Ok(())
}

trait EndpointExt {
    fn config_endpoint(&self) -> Option<String>;
}

impl EndpointExt for InvokeCtx<'_> {
    fn config_endpoint(&self) -> Option<String> {
        self.integration
            .config
            .get("endpoint")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }
}

fn apply_auth(mut req: reqwest::RequestBuilder, ctx: &InvokeCtx<'_>) -> reqwest::RequestBuilder {
    let method = ctx
        .integration
        .integration
        .auth_methods
        .iter()
        .find(|m| m.template.as_str() == ctx.template.as_str() || m.id == ctx.template.as_str())
        .cloned()
        .unwrap_or_else(AuthMethod::none);
    if (method.kind == AuthKind::Oauth || method.kind == AuthKind::Header)
        && let Some(token) = ctx.values.get("token")
    {
        req = req.bearer_auth(token);
    }
    for p in &method.placements {
        if p.carrier == executor_core::Carrier::Header
            && let Some(v) = p
                .literal
                .clone()
                .or_else(|| ctx.values.get(&p.variable).cloned())
        {
            req = req.header(&p.name, format!("{}{v}", p.prefix));
        }
    }
    req
}

const INTROSPECTION_QUERY: &str = r"query IntrospectionQuery { __schema { queryType { name } mutationType { name } types { name kind fields { name description args { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } enumValues { name } inputFields { name type { kind name ofType { kind name ofType { kind name } } } } } } }";

async fn introspect(
    client: &Client,
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    timeout: Duration,
) -> Result<Value, PluginError> {
    let endpoint = config
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::new("missing endpoint"))?;
    let mut req = client.post(endpoint).timeout(timeout).json(&json!({
        "query": INTROSPECTION_QUERY,
    }));
    if let Some(token) = values.get("token") {
        req = req.bearer_auth(token);
    }
    let response = req
        .send()
        .await
        .map_err(|e| PluginError::new(format!("introspect: {e}")))?;
    let body: Value = response
        .json()
        .await
        .map_err(|e| PluginError::new(format!("introspect body: {e}")))?;
    body.get("data")
        .cloned()
        .ok_or_else(|| PluginError::new("introspection returned no data"))
}

#[cfg(test)]
mod tests {
    use super::GraphqlPlugin;
    use executor_core::{AuthKind, IntegrationPlugin};
    use serde_json::json;

    #[test]
    fn detects_graphql() {
        let p = GraphqlPlugin::new();
        assert!(p.detect("https://api.example.test/graphql").is_some());
        assert!(p.detect("https://example.test/v1").is_none());
    }

    #[test]
    fn describe_auth_includes_oauth() {
        let p = GraphqlPlugin::new();
        let methods = p.describe_auth(&json!({
            "authorizationUrl": "https://auth.example/authorize",
            "tokenUrl": "https://auth.example/token",
            "scopes": ["read"]
        }));
        assert!(methods.iter().any(|m| m.kind == AuthKind::Oauth));
    }
}
