//! Secret references. Values never appear in tool I/O schemas.

use serde::{Deserialize, Serialize};

use crate::{ProviderItemId, ProviderKey};

/// Pointer to a secret. The value is resolved in trusted space at call time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SecretRef {
    /// `ENV_VAR` in the process environment.
    Env {
        /// Variable name.
        name: String,
    },
    /// File contents (trimmed) at `path`.
    File {
        /// Filesystem path.
        path: String,
    },
    /// Provider-backed item.
    Provider {
        /// Backend key.
        provider: ProviderKey,
        /// Opaque item id.
        item: ProviderItemId,
    },
    /// In-memory test / default store id.
    Memory {
        /// Item id in the memory map.
        id: String,
    },
}

impl SecretRef {
    /// Parse `env:NAME`, `file:PATH`, `memory:ID`, or `provider:KEY:ITEM`.
    ///
    /// # Errors
    ///
    /// Unknown scheme or missing parts.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if let Some(name) = raw.strip_prefix("env:") {
            if name.is_empty() {
                return Err("env: requires a variable name".to_owned());
            }
            return Ok(Self::Env {
                name: name.to_owned(),
            });
        }
        if let Some(path) = raw.strip_prefix("file:") {
            if path.is_empty() {
                return Err("file: requires a path".to_owned());
            }
            return Ok(Self::File {
                path: path.to_owned(),
            });
        }
        if let Some(id) = raw.strip_prefix("memory:") {
            if id.is_empty() {
                return Err("memory: requires an id".to_owned());
            }
            return Ok(Self::Memory { id: id.to_owned() });
        }
        if let Some(rest) = raw.strip_prefix("provider:") {
            let (provider, item) = rest
                .split_once(':')
                .ok_or_else(|| "provider: requires provider:item".to_owned())?;
            return Ok(Self::Provider {
                provider: ProviderKey::new(provider).map_err(|e| e.to_string())?,
                item: ProviderItemId::new(item).map_err(|e| e.to_string())?,
            });
        }
        Err(format!("unknown secret ref: {raw}"))
    }
}

/// Recursively drop values that look like secret refs from JSON (defense in depth).
#[must_use]
pub fn strip_secret_refs(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("kind").and_then(serde_json::Value::as_str) == Some("provider")
                || map.get("kind").and_then(serde_json::Value::as_str) == Some("env")
                    && map.contains_key("name")
            {
                return serde_json::Value::Null;
            }
            let cleaned = map
                .into_iter()
                .map(|(k, v)| (k, strip_secret_refs(v)))
                .collect();
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(strip_secret_refs).collect())
        }
        other => other,
    }
}
