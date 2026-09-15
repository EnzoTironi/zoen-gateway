//! The one open integration seam: detect, describe auth, resolve tools, invoke.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::{
    AuthMethod, AuthTemplateSlug, ConnectionRef, Detection, Integration, IntegrationConfig,
    IntegrationRecord, PluginId, ToolDef, ToolResult,
};

/// Plugin-level failure (infra, not `ToolResult::Err`).
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0}")]
pub struct PluginError(pub String);

impl PluginError {
    /// Wrap a displayable cause.
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

/// Context for producing a connection's tool list.
pub struct ResolveToolsCtx<'a> {
    /// Public integration.
    pub integration: &'a Integration,
    /// Opaque config.
    pub config: &'a IntegrationConfig,
    /// Connection identity.
    pub connection: &'a ConnectionRef,
    /// Resolved credential values (`variable → plaintext`). Empty for `none`.
    pub values: &'a CredentialMapValues,
    /// Outbound IO budget.
    pub timeout: Duration,
    /// Max tools this resolve may persist.
    pub max_tools: usize,
}

/// Resolved plaintext credential inputs. Never a [`crate::SecretRef`].
pub type CredentialMapValues = std::collections::BTreeMap<String, String>;

/// Tool listing produced by a plugin.
#[derive(Clone, Debug, Default)]
pub struct ResolvedTools {
    /// Tools (no addresses yet).
    pub tools: Vec<ToolDef>,
    /// Shared `$defs`.
    pub definitions: Option<Value>,
    /// Non-authoritative listing (keep prior catalog).
    pub incomplete: bool,
    /// Why incomplete.
    pub incomplete_reason: Option<String>,
}

/// Invoke context. Credential values are already resolved.
pub struct InvokeCtx<'a> {
    /// Catalog record.
    pub integration: &'a IntegrationRecord,
    /// Connection identity.
    pub connection: &'a ConnectionRef,
    /// Auth template the connection bound.
    pub template: &'a AuthTemplateSlug,
    /// Tool as persisted.
    pub tool: &'a crate::Tool,
    /// JSON args.
    pub args: &'a Value,
    /// Resolved secrets. Never include [`crate::SecretRef`].
    pub values: &'a CredentialMapValues,
    /// Outbound IO budget.
    pub timeout: Duration,
}

/// Health-check context.
pub struct HealthCheckCtx<'a> {
    /// Integration.
    pub integration: &'a IntegrationRecord,
    /// Resolved values.
    pub values: &'a CredentialMapValues,
    /// Outbound IO budget.
    pub timeout: Duration,
}

/// First-party and third-party integrations implement this seam.
#[async_trait]
pub trait IntegrationPlugin: Send + Sync {
    /// Stable plugin id (`openapi`, `graphql`, `mcp`).
    fn id(&self) -> PluginId;

    /// Does this plugin claim `candidate` (usually a URL)?
    fn detect(&self, candidate: &str) -> Option<Detection>;

    /// Project opaque config into catalog auth methods.
    fn describe_auth(&self, config: &IntegrationConfig) -> Vec<AuthMethod>;

    /// Enumerate tools for a connection.
    async fn resolve_tools(&self, ctx: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError>;

    /// Invoke one tool.
    async fn invoke(&self, ctx: InvokeCtx<'_>) -> Result<ToolResult, PluginError>;

    /// Optional liveness probe. Default: unknown.
    async fn check_health(
        &self,
        _ctx: HealthCheckCtx<'_>,
    ) -> Result<crate::HealthVerdict, PluginError> {
        Ok(crate::HealthVerdict::Unknown)
    }
}
