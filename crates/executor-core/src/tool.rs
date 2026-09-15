//! Tools: definitions from plugins, persisted addressable rows, list filters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ConnectionName, IntegrationSlug, Owner, PluginId, ToolAddress, ToolName};

/// Default-policy hints a plugin attaches to a tool.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    /// Plugin asks the engine to require approval when no user rule matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    /// Human description of the approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_description: Option<String>,
    /// Tool may elicit more input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub may_elicit: Option<bool>,
}

/// Plugin-produced tool, not yet addressed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDef {
    /// Tool name (may contain dots).
    pub name: ToolName,
    /// Agent-visible description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema for arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// JSON Schema for output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Policy hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    /// Plugin-private metadata (operation binding). Never agent-facing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_meta: Option<Value>,
}

/// Persisted, addressable tool.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Tool {
    /// Full address.
    pub address: ToolAddress,
    /// Owner segment.
    pub owner: Owner,
    /// Integration slug.
    pub integration: IntegrationSlug,
    /// Connection name.
    pub connection: ConnectionName,
    /// Tool name.
    pub name: ToolName,
    /// Owning plugin.
    pub plugin_id: PluginId,
    /// Description.
    pub description: String,
    /// Input schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// Output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    /// Static (built-in) tools have no backing connection even though they
    /// carry owner/connection metadata for grouping.
    #[serde(default)]
    pub static_tool: bool,
    /// Plugin-private metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_meta: Option<Value>,
}

impl Tool {
    /// Sandbox / CLI path (`integration.owner.connection.tool` or static fqid).
    #[must_use]
    pub fn cli_path(&self) -> String {
        if self.static_tool {
            self.name.as_str().to_owned()
        } else {
            self.address.sandbox_path()
        }
    }
}

/// Filter for `tools.list`.
#[derive(Clone, Debug, Default)]
pub struct ToolListFilter {
    /// Restrict to one integration.
    pub integration: Option<IntegrationSlug>,
    /// Restrict to one owner.
    pub owner: Option<Owner>,
    /// Restrict to one connection.
    pub connection: Option<ConnectionName>,
    /// Case-insensitive substring on name or description.
    pub query: Option<String>,
    /// Include blocked tools. Default false (agent-facing).
    pub include_blocked: bool,
}
