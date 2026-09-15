//! Integration catalog identity. Plugin config is an opaque JSON blob.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::id::IntegrationSlug;
use crate::{AuthMethod, PluginId};

/// Opaque plugin configuration stored on the integration row.
pub type IntegrationConfig = Value;

/// Public projection — no credentials, no plugin internals.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Integration {
    /// Catalog slug.
    pub slug: IntegrationSlug,
    /// Display name.
    pub name: String,
    /// Agent-visible description.
    pub description: String,
    /// Owning plugin id.
    pub kind: PluginId,
    /// User-removable?
    pub can_remove: bool,
    /// Supports `connections.refresh`?
    pub can_refresh: bool,
    /// Declared auth methods (derived projection).
    pub auth_methods: Vec<AuthMethod>,
    /// Non-secret display URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_url: Option<String>,
}

/// Catalog row including opaque config.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IntegrationRecord {
    /// Public projection.
    pub integration: Integration,
    /// Plugin-owned blob.
    pub config: IntegrationConfig,
}

/// Input to register / replace an integration.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RegisterIntegration {
    /// Slug.
    pub slug: IntegrationSlug,
    /// Display name (falls back to description / slug).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Description.
    pub description: String,
    /// Opaque config.
    pub config: IntegrationConfig,
    /// Removable. Default true.
    #[serde(default = "default_true")]
    pub can_remove: bool,
    /// Refreshable. Default true for spec plugins.
    #[serde(default = "default_true")]
    pub can_refresh: bool,
}

const fn default_true() -> bool {
    true
}

/// Confidence when a plugin claims a URL.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionConfidence {
    /// Spec/content-type is unambiguous.
    High,
    /// Likely, but another plugin might also claim it.
    Medium,
    /// Weak heuristic.
    Low,
}

/// `integrations.detect` hit.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Detection {
    /// Plugin id.
    pub kind: PluginId,
    /// Confidence.
    pub confidence: DetectionConfidence,
    /// Normalized endpoint.
    pub endpoint: String,
    /// Suggested display name.
    pub name: String,
    /// Suggested slug.
    pub slug: String,
}
