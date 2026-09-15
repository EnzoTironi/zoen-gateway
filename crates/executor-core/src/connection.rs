//! Connections: born-wired credentials. Values live in a secret provider.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AuthTemplateSlug, ConnectionAddress, ConnectionName, IntegrationSlug, Owner, ProviderKey,
    SecretRef,
};

/// Identify one connection: unique by `(owner, integration, name)`.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ConnectionRef {
    /// Owner.
    pub owner: Owner,
    /// Connection name.
    pub name: ConnectionName,
    /// Integration.
    pub integration: IntegrationSlug,
}

impl ConnectionRef {
    /// Display as `owner/integration/name`.
    #[must_use]
    pub fn as_key(&self) -> String {
        format!("{}/{}/{}", self.owner, self.integration, self.name)
    }
}

/// Optional human label for which account this is.
pub type IdentityLabel = String;

/// Last health-check verdict.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthVerdict {
    /// Probe succeeded.
    Healthy {
        /// Optional identity string from the probe.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<String>,
    },
    /// Probe failed.
    Unhealthy {
        /// Why.
        message: String,
    },
    /// Never checked / unknown.
    Unknown,
}

/// Saved credential. The secret is a ref, never a value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Connection {
    /// Owner.
    pub owner: Owner,
    /// Name.
    pub name: ConnectionName,
    /// Wired integration.
    pub integration: IntegrationSlug,
    /// Auth template this connection binds.
    pub template: AuthTemplateSlug,
    /// Secret backend.
    pub provider: ProviderKey,
    /// Callable handle.
    pub address: ConnectionAddress,
    /// Optional account label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_label: Option<String>,
    /// Curated description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Named secret refs (`token` for single-secret methods).
    pub secrets: CredentialMap,
    /// Last health.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_health: Option<HealthVerdict>,
}

/// Variable → secret ref.
pub type CredentialMap = BTreeMap<String, SecretRef>;

/// Create-connection input. Values are written to the default provider; refs stay refs.
#[derive(Clone, Debug)]
pub struct ConnectionInput {
    /// Owner.
    pub owner: Owner,
    /// Name.
    pub name: ConnectionName,
    /// Integration.
    pub integration: IntegrationSlug,
    /// Template.
    pub template: AuthTemplateSlug,
    /// Optional label.
    pub identity_label: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Pasted values (stored, never returned).
    pub values: BTreeMap<String, String>,
    /// External refs (not pasted).
    pub refs: CredentialMap,
}
