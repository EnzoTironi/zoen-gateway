//! Resolve [`executor_core::SecretRef`] in trusted space. Values never enter tool I/O.

use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use executor_core::{ExecutorError, ProviderItemId, ProviderKey, SecretRef, StorageError};
use parking_lot::RwLock;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

impl MemorySecrets {
    fn load_map(&self, items: BTreeMap<String, String>) {
        let max = items
            .keys()
            .filter_map(|k| k.strip_prefix("sec_")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        ITEM_SEQ.fetch_max(max.saturating_add(1), Ordering::Relaxed);
        *self.items.write() = items;
    }

    fn dump_map(&self) -> BTreeMap<String, String> {
        self.items.read().clone()
    }
}

/// Default store persisted as owner-only JSON next to the catalog.
pub struct FileSecrets {
    path: std::path::PathBuf,
    inner: MemorySecrets,
}

impl FileSecrets {
    /// Load `path` if it exists; otherwise start empty.
    ///
    /// # Errors
    ///
    /// Unreadable or illegal JSON.
    pub fn open(path: impl Into<std::path::PathBuf>) -> Result<Self, ExecutorError> {
        let path = path.into();
        let inner = MemorySecrets::new();
        if path.is_file() {
            let text = fs::read_to_string(&path).map_err(StorageError::new)?;
            let map: BTreeMap<String, String> =
                serde_json::from_str(&text).map_err(StorageError::new)?;
            inner.load_map(map);
        }
        Ok(Self { path, inner })
    }

    fn persist(&self) -> Result<(), ExecutorError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(StorageError::new)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(&self.inner.dump_map()).map_err(StorageError::new)?;
        fs::write(&tmp, bytes).map_err(StorageError::new)?;
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&tmp).map_err(StorageError::new)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&tmp, perms).map_err(StorageError::new)?;
        }
        fs::rename(&tmp, &self.path).map_err(StorageError::new)?;
        Ok(())
    }
}

impl SecretResolver for FileSecrets {
    fn resolve(&self, secret: &SecretRef) -> Result<String, ExecutorError> {
        self.inner.resolve(secret)
    }

    fn store_default(&self, value: &str) -> Result<SecretRef, ExecutorError> {
        let stored = self.inner.store_default(value)?;
        self.persist()?;
        Ok(stored)
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

    #[test]
    fn file_store_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let first = super::FileSecrets::open(&path).unwrap();
        let r = first.store_default("disk-secret").unwrap();
        drop(first);
        let second = super::FileSecrets::open(&path).unwrap();
        assert_eq!(second.resolve(&r).unwrap(), "disk-secret");
    }
}
