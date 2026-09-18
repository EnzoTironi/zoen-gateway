//! HTTP spec (`OpenAPI` 3 / Swagger 2 / Google Discovery) integration plugin.

#![allow(clippy::module_name_repetitions)] // `OpenApiPlugin` is the crate's type.

mod extract;
mod invoke;
mod presets;

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use executor_core::{
    AuthKind, AuthMethod, CredentialMapValues, Detection, DetectionConfidence, HealthCheckCtx,
    HealthVerdict, IntegrationConfig, IntegrationPlugin, InvokeCtx, PluginError, PluginId,
    ResolveToolsCtx, ResolvedTools, ToolResult,
};
use reqwest::Client;
use serde_json::Value;
use tracing::instrument;

pub use extract::{
    extract_operations, operations_by_tag, parse_spec, spec_base_url, tools_from_operations,
};
pub use presets::{
    GOOGLE_PRESETS, GRAPH_SCOPE_PRESETS, MICROSOFT_GRAPH_OPENAPI_URL, apply_preset, google_preset,
    google_preset_for_url, graph_filters, graph_path_kept, graph_preset, is_graph_monolith_url,
    is_graph_url,
};

/// First-party `OpenAPI` plugin. Shared HTTP client (connection pool).
pub struct OpenApiPlugin {
    client: Client,
}

impl OpenApiPlugin {
    /// Build with a pooled rustls client.
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(32)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }
}

impl Default for OpenApiPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IntegrationPlugin for OpenApiPlugin {
    fn id(&self) -> PluginId {
        PluginId::openapi()
    }

    fn detect(&self, candidate: &str) -> Option<Detection> {
        if presets::is_graph_url(candidate) {
            return Some(Detection {
                kind: PluginId::openapi(),
                confidence: DetectionConfidence::High,
                endpoint: candidate.to_owned(),
                name: "Microsoft Graph".into(),
                slug: "microsoft".into(),
            });
        }
        if let Some(google) = presets::google_preset_for_url(candidate) {
            return Some(Detection {
                kind: PluginId::openapi(),
                confidence: DetectionConfidence::High,
                endpoint: google.url.to_owned(),
                name: google.name.to_owned(),
                slug: google.id.replace('-', "_"),
            });
        }
        let lower = candidate.to_ascii_lowercase();
        let high = lower.contains("openapi")
            || lower.contains("swagger")
            || lower.contains("$discovery")
            || lower.contains("discovery/v1");
        let spec_file = ["yaml", "yml", "json"].iter().any(|ext| {
            Path::new(candidate)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        });
        if high || spec_file {
            Some(Detection {
                kind: PluginId::openapi(),
                confidence: if high {
                    DetectionConfidence::High
                } else {
                    DetectionConfidence::Medium
                },
                endpoint: candidate.to_owned(),
                name: "OpenAPI".into(),
                slug: "openapi".into(),
            })
        } else {
            None
        }
    }

    fn describe_auth(&self, config: &IntegrationConfig) -> Vec<AuthMethod> {
        with_preset(config).map_or_else(
            |_| describe_from_config(config),
            |expanded| describe_from_config(&expanded),
        )
    }

    #[instrument(skip(self, ctx))]
    async fn resolve_tools(&self, ctx: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError> {
        let config = with_preset(ctx.config)?;
        let text = load_spec(&self.client, &config, ctx.timeout).await?;
        if text.len() > ctx.max_spec_bytes {
            return Err(PluginError::new(format!(
                "spec is {} bytes (max {})",
                text.len(),
                ctx.max_spec_bytes
            )));
        }
        let doc = parse_spec(&text)?;
        let mut ops = extract_operations(&doc)?;
        if let Some(tag) = config.get("tag").and_then(Value::as_str) {
            ops.retain(|op| op.tag.as_deref() == Some(tag));
        }
        let (prefixes, exact) = presets::graph_filters(&config);
        if !prefixes.is_empty() || !exact.is_empty() {
            ops.retain(|op| presets::graph_path_kept(&op.path, &prefixes, &exact));
        }
        if ops.len() > ctx.max_tools {
            return Err(PluginError::new(format!(
                "spec produced {} operations (max {})",
                ops.len(),
                ctx.max_tools
            )));
        }
        let tools = tools_from_operations(&ops, spec_base_url(&doc).as_deref())?;
        Ok(ResolvedTools {
            tools,
            definitions: None,
            incomplete: false,
            incomplete_reason: None,
        })
    }

    #[instrument(skip(self, ctx))]
    async fn invoke(&self, ctx: InvokeCtx<'_>) -> Result<ToolResult, PluginError> {
        let meta = ctx
            .tool
            .plugin_meta
            .as_ref()
            .ok_or_else(|| PluginError::new("missing plugin_meta"))?;
        let spec_base = meta
            .get("baseUrl")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                ctx.integration
                    .config
                    .get("spec")
                    .and_then(Value::as_str)
                    .and_then(|t| parse_spec(t).ok().and_then(|d| spec_base_url(&d)))
            });
        let config_base = ctx
            .integration
            .config
            .get("baseUrl")
            .and_then(Value::as_str);
        let base = invoke::effective_base(config_base, spec_base.as_deref());
        let (kind, placements) = bound_auth(
            &ctx.integration.integration.auth_methods,
            ctx.template.as_str(),
            ctx.values,
        );
        let static_headers = ctx
            .integration
            .config
            .get("headers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let headers = invoke::render_headers(&placements, ctx.values, &static_headers, kind)?;
        let mut query = invoke::auth_query(&placements, ctx.values);
        if let Some(obj) = ctx
            .integration
            .config
            .get("queryParams")
            .and_then(Value::as_object)
        {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    query.push((k.clone(), s.to_owned()));
                }
            }
        }
        invoke::invoke_operation(
            &self.client,
            &base,
            meta,
            ctx.args,
            headers,
            &query,
            ctx.timeout,
        )
        .await
    }

    async fn check_health(&self, _ctx: HealthCheckCtx<'_>) -> Result<HealthVerdict, PluginError> {
        Ok(HealthVerdict::Unknown)
    }
}

fn with_preset(config: &IntegrationConfig) -> Result<Value, PluginError> {
    let mut cfg = config.clone();
    let Some(id) = cfg
        .get("preset")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    else {
        return Ok(cfg);
    };
    if let Some(obj) = cfg.as_object_mut() {
        presets::apply_preset(obj, &id)?;
    }
    Ok(cfg)
}

fn describe_from_config(config: &IntegrationConfig) -> Vec<AuthMethod> {
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
    if let Some(oauth) = oauth_from_config(config) {
        methods.push(oauth);
    }
    methods
}

fn oauth_from_config(config: &IntegrationConfig) -> Option<AuthMethod> {
    let auth = config.get("authorizationUrl").and_then(Value::as_str)?;
    let token = config.get("tokenUrl").and_then(Value::as_str)?;
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
    Some(AuthMethod::oauth(auth, token, scopes))
}

async fn load_spec(
    client: &Client,
    config: &IntegrationConfig,
    timeout: Duration,
) -> Result<String, PluginError> {
    if let Some(text) = config.get("spec").and_then(Value::as_str) {
        return Ok(text.to_owned());
    }
    if let Some(obj) = config.get("spec").and_then(Value::as_object) {
        return Ok(serde_json::to_string(obj).unwrap_or_else(|_| "{}".into()));
    }
    let url = config
        .get("specUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::new("integration config has neither spec nor specUrl"))?;
    if presets::is_graph_monolith_url(url) {
        return Err(PluginError::new(
            "refusing to fetch the Microsoft Graph OpenAPI monolith (~43MB). Pass a sliced spec and a Graph preset id (mail, calendar, files, …).",
        ));
    }
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| PluginError::new(format!("fetch spec: {e}")))?;
    if !response.status().is_success() {
        return Err(PluginError::new(format!(
            "fetch spec: HTTP {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|e| PluginError::new(format!("fetch spec body: {e}")))
}

/// Apply credential values using the connection's bound template when listed.
#[must_use]
pub fn bound_auth(
    methods: &[AuthMethod],
    template: &str,
    values: &CredentialMapValues,
) -> (AuthKind, Vec<executor_core::AuthPlacement>) {
    let _ = values;
    let method = methods
        .iter()
        .find(|m| m.template.as_str() == template || m.id == template)
        .cloned()
        .unwrap_or_else(|| {
            if methods.len() == 1 {
                methods[0].clone()
            } else {
                methods
                    .iter()
                    .find(|m| m.kind != AuthKind::None)
                    .cloned()
                    .unwrap_or_else(AuthMethod::none)
            }
        });
    (method.kind, method.placements)
}

#[cfg(test)]
mod tests {
    use super::OpenApiPlugin;
    use executor_core::IntegrationPlugin;

    #[test]
    fn detects_openapi_urls() {
        let p = OpenApiPlugin::new();
        assert!(p.detect("https://example.test/openapi.json").is_some());
        let gmail = p
            .detect("https://www.googleapis.com/discovery/v1/apis/gmail/v1/rest")
            .expect("gmail");
        assert_eq!(gmail.name, "Gmail");
        let graph = p.detect("https://graph.microsoft.com/v1.0").expect("graph");
        assert_eq!(graph.slug, "microsoft");
    }

    #[tokio::test]
    async fn mail_preset_filters_graph_paths() {
        use std::collections::BTreeMap;
        use std::time::Duration;

        use executor_core::{
            ConnectionName, ConnectionRef, Integration, IntegrationSlug, Owner, PluginId,
            ResolveToolsCtx,
        };

        use super::presets::apply_preset;

        let spec = serde_json::json!({
            "openapi": "3.0.0",
            "info": {"title": "g", "version": "1"},
            "paths": {
                "/me/messages": {"get": {
                    "operationId": "listMail",
                    "responses": {"200": {"description": "ok"}}
                }},
                "/sites": {"get": {
                    "operationId": "listSites",
                    "responses": {"200": {"description": "ok"}}
                }}
            }
        });
        let mut config = serde_json::Map::new();
        config.insert("spec".into(), spec);
        apply_preset(&mut config, "mail").expect("preset");
        let config = serde_json::Value::Object(config);
        let integration = Integration {
            slug: IntegrationSlug::new("microsoft").expect("slug"),
            name: "Graph".into(),
            description: String::new(),
            kind: PluginId::openapi(),
            can_remove: true,
            can_refresh: true,
            auth_methods: vec![],
            display_url: None,
        };
        let connection = ConnectionRef {
            owner: Owner::Org,
            name: ConnectionName::new("work").expect("name"),
            integration: IntegrationSlug::new("microsoft").expect("slug"),
        };
        let values = BTreeMap::new();
        let ctx = ResolveToolsCtx {
            integration: &integration,
            config: &config,
            connection: &connection,
            values: &values,
            timeout: Duration::from_secs(1),
            max_tools: 100,
            max_spec_bytes: 1_000_000,
        };
        let resolved = OpenApiPlugin::new()
            .resolve_tools(ctx)
            .await
            .expect("resolve");
        let names: Vec<String> = resolved
            .tools
            .iter()
            .map(|t| t.name.as_str().to_ascii_lowercase())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("mail") || n.contains("listmail")),
            "{names:?}"
        );
        assert!(names.iter().all(|n| !n.contains("sites")), "{names:?}");
    }

    #[tokio::test]
    async fn refuses_graph_monolith_url() {
        use std::collections::BTreeMap;
        use std::time::Duration;

        use executor_core::{
            ConnectionName, ConnectionRef, Integration, IntegrationSlug, Owner, PluginId,
            ResolveToolsCtx,
        };

        use super::MICROSOFT_GRAPH_OPENAPI_URL;

        let config = serde_json::json!({"specUrl": MICROSOFT_GRAPH_OPENAPI_URL});
        let integration = Integration {
            slug: IntegrationSlug::new("microsoft").expect("slug"),
            name: "Graph".into(),
            description: String::new(),
            kind: PluginId::openapi(),
            can_remove: true,
            can_refresh: true,
            auth_methods: vec![],
            display_url: None,
        };
        let connection = ConnectionRef {
            owner: Owner::Org,
            name: ConnectionName::new("work").expect("name"),
            integration: IntegrationSlug::new("microsoft").expect("slug"),
        };
        let values = BTreeMap::new();
        let ctx = ResolveToolsCtx {
            integration: &integration,
            config: &config,
            connection: &connection,
            values: &values,
            timeout: Duration::from_secs(1),
            max_tools: 100,
            max_spec_bytes: 1_000_000,
        };
        let err = OpenApiPlugin::new()
            .resolve_tools(ctx)
            .await
            .expect_err("monolith");
        assert!(
            err.to_string().contains("monolith") || err.to_string().contains("43"),
            "{err}"
        );
    }
}
