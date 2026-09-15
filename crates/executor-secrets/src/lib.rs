//! Resolve [`executor_core::SecretRef`] in trusted space. Values never enter tool I/O.

use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use executor_core::{ExecutorError, ProviderItemId, ProviderKey, SecretRef};
use parking_lot::RwLock;

static ITEM_SEQ: AtomicU64 = AtomicU64::new(1);

/// Resolve and (for the default store) persist secrets.
pub trait SecretResolver: Send + Sync {
    /// Read the plaintext for a ref.
    ///
    /// # Errors
    ///
    /// Missing provider, missing item, or IO.
    fn resolve(&self, secret: &SecretRef) -> Result<String, ExecutorError>;

    /// Store a pasted value in the default writable backend.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn store_default(&self, value: &str) -> Result<SecretRef, ExecutorError>;
}

/// In-memory default store plus env/file resolvers.
#[derive(Default)]
pub struct MemorySecrets {
    items: RwLock<BTreeMap<String, String>>,
}

impl MemorySecrets {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretResolver for MemorySecrets {
    fn resolve(&self, secret: &SecretRef) -> Result<String, ExecutorError> {
        match secret {
            SecretRef::Memory { id } => self.items.read().get(id).cloned().ok_or_else(|| {
                ExecutorError::CredentialResolution(format!("missing memory id {id}"))
            }),
            SecretRef::Env { name } => std::env::var(name)
                .map_err(|_| ExecutorError::CredentialResolution(format!("missing env {name}"))),
            SecretRef::File { path } => fs::read_to_string(path)
                .map(|s| s.trim().to_owned())
                .map_err(|e| ExecutorError::CredentialResolution(e.to_string())),
            SecretRef::Provider { provider, item } => {
                if provider.as_str() == "default" || provider.as_str() == "memory" {
                    self.items
                        .read()
                        .get(item.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            ExecutorError::CredentialResolution(format!("missing item {item}"))
                        })
                } else if provider.as_str() == "env" {
                    std::env::var(item.as_str()).map_err(|_| {
                        ExecutorError::CredentialResolution(format!("missing env {item}"))
                    })
                } else {
                    Err(ExecutorError::ProviderNotRegistered(provider.to_string()))
                }
            }
        }
    }

    fn store_default(&self, value: &str) -> Result<SecretRef, ExecutorError> {
        let id = format!("sec_{}", ITEM_SEQ.fetch_add(1, Ordering::Relaxed));
        self.items.write().insert(id.clone(), value.to_owned());
        Ok(SecretRef::Provider {
            provider: ProviderKey::default_store(),
            item: ProviderItemId::new(&id).map_err(ExecutorError::from)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{MemorySecrets, SecretResolver};
    use executor_core::SecretRef;

    #[test]
    fn roundtrip_default() {
        let store = MemorySecrets::new();
        let r = store.store_default("s3cret").unwrap();
        assert_eq!(store.resolve(&r).unwrap(), "s3cret");
    }

    #[test]
    fn env_ref() {
        let store = MemorySecrets::new();
        let value = store
            .resolve(&SecretRef::Env {
                name: "PATH".into(),
            })
            .unwrap();
        assert!(!value.is_empty());
    }
}
