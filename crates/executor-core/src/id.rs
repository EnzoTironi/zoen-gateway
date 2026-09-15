//! Branded identifiers. Construction is the only validation gate.

use std::fmt::{Display, Formatter, Result as FmtResult};

use serde::{Deserialize, Serialize};

use crate::InvalidId;

macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parse a non-empty identifier.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidId`] when `raw` is empty after trim.
            pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidId> {
                let value = raw.as_ref().trim();
                if value.is_empty() {
                    return Err(InvalidId::new($kind, raw.as_ref(), "must be non-empty"));
                }
                Ok(Self(value.to_owned()))
            }

            /// Borrow the raw text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

/// Slug-like ids: non-empty, no `.` (so address segments stay unambiguous).
macro_rules! slug_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parse a slug segment.
            ///
            /// # Errors
            ///
            /// Empty, containing `.`, or starting/ending with `.`.
            pub fn new(raw: impl AsRef<str>) -> Result<Self, InvalidId> {
                let value = raw.as_ref().trim();
                if value.is_empty() {
                    return Err(InvalidId::new($kind, raw.as_ref(), "must be non-empty"));
                }
                if value.contains('.') {
                    return Err(InvalidId::new($kind, value, "must not contain '.'"));
                }
                Ok(Self(value.to_owned()))
            }

            /// Borrow the raw text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

slug_id!(
    /// Catalog slug for one API surface (`vercel`, `github`).
    IntegrationSlug,
    "integration slug"
);
slug_id!(
    /// Connection name — the account (`work`, `personal`, `prod`).
    ConnectionName,
    "connection name"
);
slug_id!(
    /// Plugin id (`openapi`, `graphql`, `mcp`, `core-tools`).
    PluginId,
    "plugin id"
);
slug_id!(
    /// Secret backend key (`default`, `env`, `file`).
    ProviderKey,
    "provider key"
);

opaque_id!(
    /// Tool name. May contain `.` (`aliases.deleteAlias`).
    ToolName,
    "tool name"
);
opaque_id!(
    /// Opaque provider item handle. Core never parses it.
    ProviderItemId,
    "provider item id"
);
opaque_id!(
    /// Tool-policy rule id.
    PolicyId,
    "policy id"
);
opaque_id!(
    /// Isolation partition (org/workspace). Opaque to plugins.
    Tenant,
    "tenant"
);
opaque_id!(
    /// Acting member identity. Required for `Owner::User` writes.
    Subject,
    "subject"
);
opaque_id!(
    /// Registered OAuth app slug.
    OAuthClientSlug,
    "oauth client slug"
);
opaque_id!(
    /// URL elicitation correlation id.
    ElicitationId,
    "elicitation id"
);
opaque_id!(
    /// Generative-UI artifact id (kept for address compatibility; UI is not ported).
    ArtifactId,
    "artifact id"
);

impl PluginId {
    /// First-party `openapi` plugin.
    #[must_use]
    pub fn openapi() -> Self {
        Self("openapi".to_owned())
    }

    /// First-party `graphql` plugin.
    #[must_use]
    pub fn graphql() -> Self {
        Self("graphql".to_owned())
    }

    /// First-party `mcp` client plugin.
    #[must_use]
    pub fn mcp() -> Self {
        Self("mcp".to_owned())
    }

    /// Built-in core-tools plugin.
    #[must_use]
    pub fn core_tools() -> Self {
        Self("core-tools".to_owned())
    }
}

impl ProviderKey {
    /// Default writable store (sqlite or memory).
    #[must_use]
    pub fn default_store() -> Self {
        Self("default".to_owned())
    }

    /// Process environment.
    #[must_use]
    pub fn env() -> Self {
        Self("env".to_owned())
    }

    /// File path provider.
    #[must_use]
    pub fn file() -> Self {
        Self("file".to_owned())
    }

    /// 1Password CLI (`op read`).
    #[must_use]
    pub fn one_password() -> Self {
        Self("1password".to_owned())
    }

    /// OS keychain / `secret-tool`.
    #[must_use]
    pub fn keychain() -> Self {
        Self("keychain".to_owned())
    }

    /// `WorkOS` Vault HTTP.
    #[must_use]
    pub fn workos_vault() -> Self {
        Self("workos_vault".to_owned())
    }
}
