//! Catalog persistence. Implementations live here (memory) or in `executor-storage`.

use std::collections::BTreeMap;

use parking_lot::RwLock;

use crate::{
    Connection, ConnectionRef, ExecutionId, ExecutionState, IntegrationRecord, IntegrationSlug,
    Owner, PolicyId, StorageError, Tool, ToolAddress, ToolPolicy,
};

/// Durable catalog. All methods are synchronous; IO adapters wrap internally.
pub trait CatalogStore: Send + Sync {
    /// Upsert an integration (replace config on the same slug).
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_integration(&self, row: IntegrationRecord) -> Result<(), StorageError>;

    /// Fetch one integration.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_integration(
        &self,
        slug: &IntegrationSlug,
    ) -> Result<Option<IntegrationRecord>, StorageError>;

    /// List all integrations.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn list_integrations(&self) -> Result<Vec<IntegrationRecord>, StorageError>;

    /// Remove an integration. Returns whether a row existed.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn remove_integration(&self, slug: &IntegrationSlug) -> Result<bool, StorageError>;

    /// Upsert a connection.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_connection(&self, row: Connection) -> Result<(), StorageError>;

    /// Fetch a connection.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_connection(&self, id: &ConnectionRef) -> Result<Option<Connection>, StorageError>;

    /// List connections, optionally filtered.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn list_connections(
        &self,
        integration: Option<&IntegrationSlug>,
        owner: Option<Owner>,
    ) -> Result<Vec<Connection>, StorageError>;

    /// Remove a connection.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn remove_connection(&self, id: &ConnectionRef) -> Result<bool, StorageError>;

    /// Replace the persisted tools for one connection.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn replace_tools(&self, id: &ConnectionRef, tools: Vec<Tool>) -> Result<(), StorageError>;

    /// Fetch one tool.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_tool(&self, address: &ToolAddress) -> Result<Option<Tool>, StorageError>;

    /// List all tools.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn list_tools(&self) -> Result<Vec<Tool>, StorageError>;

    /// Upsert a policy.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_policy(&self, row: ToolPolicy) -> Result<(), StorageError>;

    /// List policies.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn list_policies(&self) -> Result<Vec<ToolPolicy>, StorageError>;

    /// Remove a policy by id.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn remove_policy(&self, id: &PolicyId) -> Result<bool, StorageError>;

    /// Persist an execution (paused/completed).
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_execution(&self, id: &ExecutionId, state: ExecutionState) -> Result<(), StorageError>;

    /// Load an execution.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError>;

    /// Delete an execution (consumed resume).
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn take_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError>;

    /// Record an idempotency key → execution id mapping (first writer wins).
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_idempotency(&self, key: &str, id: &ExecutionId) -> Result<(), StorageError>;

    /// Look up a previous execution for an idempotency key.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_idempotency(&self, key: &str) -> Result<Option<ExecutionId>, StorageError>;

    /// Upsert a JSON document in a named collection (`toolkits`, `oauth_clients`, …).
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put_kv(
        &self,
        collection: &str,
        id: &str,
        body: serde_json::Value,
    ) -> Result<(), StorageError>;

    /// Fetch one JSON document.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get_kv(&self, collection: &str, id: &str)
    -> Result<Option<serde_json::Value>, StorageError>;

    /// List documents in a collection.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn list_kv(&self, collection: &str) -> Result<Vec<serde_json::Value>, StorageError>;

    /// Delete one document. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn delete_kv(&self, collection: &str, id: &str) -> Result<bool, StorageError>;
}

/// Optional blob namespace (pending approvals, spec bodies).
pub trait BlobStore: Send + Sync {
    /// Write a blob.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn put(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), StorageError>;

    /// Read a blob.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Delete a blob.
    ///
    /// # Errors
    ///
    /// Backend failure.
    fn delete(&self, namespace: &str, key: &str) -> Result<(), StorageError>;
}

#[derive(Default)]
struct MemoryInner {
    integrations: BTreeMap<String, IntegrationRecord>,
    connections: BTreeMap<String, Connection>,
    tools: BTreeMap<String, Tool>,
    policies: BTreeMap<String, ToolPolicy>,
    executions: BTreeMap<String, ExecutionState>,
    idempotency: BTreeMap<String, ExecutionId>,
    blobs: BTreeMap<(String, String), Vec<u8>>,
    kv: BTreeMap<(String, String), serde_json::Value>,
}

/// In-process catalog. Used by tests and as the default when no sqlite path is set.
#[derive(Default)]
pub struct MemoryCatalog {
    inner: RwLock<MemoryInner>,
}

impl MemoryCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn conn_key(id: &ConnectionRef) -> String {
        id.as_key()
    }

    #[allow(clippy::suspicious_operation_groupings)] // `ConnectionRef.name` is the connection name.
    fn tool_belongs(tool: &Tool, id: &ConnectionRef) -> bool {
        tool.owner == id.owner
            && tool.integration.as_str() == id.integration.as_str()
            && tool.connection.as_str() == id.name.as_str()
    }
}

impl CatalogStore for MemoryCatalog {
    fn put_integration(&self, row: IntegrationRecord) -> Result<(), StorageError> {
        self.inner
            .write()
            .integrations
            .insert(row.integration.slug.to_string(), row);
        Ok(())
    }

    fn get_integration(
        &self,
        slug: &IntegrationSlug,
    ) -> Result<Option<IntegrationRecord>, StorageError> {
        Ok(self.inner.read().integrations.get(slug.as_str()).cloned())
    }

    fn list_integrations(&self) -> Result<Vec<IntegrationRecord>, StorageError> {
        Ok(self.inner.read().integrations.values().cloned().collect())
    }

    fn remove_integration(&self, slug: &IntegrationSlug) -> Result<bool, StorageError> {
        let mut inner = self.inner.write();
        let existed = inner.integrations.remove(slug.as_str()).is_some();
        let needle = format!("/{slug}/");
        inner.connections.retain(|k, _| !k.contains(&needle));
        inner
            .tools
            .retain(|_, t| t.integration.as_str() != slug.as_str());
        drop(inner);
        Ok(existed)
    }

    fn put_connection(&self, row: Connection) -> Result<(), StorageError> {
        let key = ConnectionRef {
            owner: row.owner,
            name: row.name.clone(),
            integration: row.integration.clone(),
        };
        self.inner
            .write()
            .connections
            .insert(Self::conn_key(&key), row);
        Ok(())
    }

    fn get_connection(&self, id: &ConnectionRef) -> Result<Option<Connection>, StorageError> {
        Ok(self
            .inner
            .read()
            .connections
            .get(&Self::conn_key(id))
            .cloned())
    }

    fn list_connections(
        &self,
        integration: Option<&IntegrationSlug>,
        owner: Option<Owner>,
    ) -> Result<Vec<Connection>, StorageError> {
        Ok(self
            .inner
            .read()
            .connections
            .values()
            .filter(|c| integration.is_none_or(|s| c.integration.as_str() == s.as_str()))
            .filter(|c| owner.is_none_or(|o| c.owner == o))
            .cloned()
            .collect())
    }

    fn remove_connection(&self, id: &ConnectionRef) -> Result<bool, StorageError> {
        let mut inner = self.inner.write();
        let existed = inner.connections.remove(&Self::conn_key(id)).is_some();
        inner.tools.retain(|_, t| !Self::tool_belongs(t, id));
        drop(inner);
        Ok(existed)
    }

    fn replace_tools(&self, id: &ConnectionRef, tools: Vec<Tool>) -> Result<(), StorageError> {
        let mut inner = self.inner.write();
        inner
            .tools
            .retain(|_, t| !Self::tool_belongs(t, id) || t.static_tool);
        for tool in tools {
            inner.tools.insert(tool.address.to_string(), tool);
        }
        drop(inner);
        Ok(())
    }

    fn get_tool(&self, address: &ToolAddress) -> Result<Option<Tool>, StorageError> {
        Ok(self.inner.read().tools.get(&address.to_string()).cloned())
    }

    fn list_tools(&self) -> Result<Vec<Tool>, StorageError> {
        Ok(self.inner.read().tools.values().cloned().collect())
    }

    fn put_policy(&self, row: ToolPolicy) -> Result<(), StorageError> {
        self.inner
            .write()
            .policies
            .insert(row.id.as_str().to_owned(), row);
        Ok(())
    }

    fn list_policies(&self) -> Result<Vec<ToolPolicy>, StorageError> {
        Ok(self.inner.read().policies.values().cloned().collect())
    }

    fn remove_policy(&self, id: &PolicyId) -> Result<bool, StorageError> {
        Ok(self.inner.write().policies.remove(id.as_str()).is_some())
    }

    fn put_execution(&self, id: &ExecutionId, state: ExecutionState) -> Result<(), StorageError> {
        self.inner
            .write()
            .executions
            .insert(id.as_str().to_owned(), state);
        Ok(())
    }

    fn get_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError> {
        Ok(self.inner.read().executions.get(id.as_str()).cloned())
    }

    fn take_execution(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, StorageError> {
        Ok(self.inner.write().executions.remove(id.as_str()))
    }

    fn put_idempotency(&self, key: &str, id: &ExecutionId) -> Result<(), StorageError> {
        let mut inner = self.inner.write();
        inner
            .idempotency
            .entry(key.to_owned())
            .or_insert_with(|| id.clone());
        drop(inner);
        Ok(())
    }

    fn get_idempotency(&self, key: &str) -> Result<Option<ExecutionId>, StorageError> {
        Ok(self.inner.read().idempotency.get(key).cloned())
    }

    fn put_kv(
        &self,
        collection: &str,
        id: &str,
        body: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .write()
            .kv
            .insert((collection.to_owned(), id.to_owned()), body);
        Ok(())
    }

    fn get_kv(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        Ok(self
            .inner
            .read()
            .kv
            .get(&(collection.to_owned(), id.to_owned()))
            .cloned())
    }

    fn list_kv(&self, collection: &str) -> Result<Vec<serde_json::Value>, StorageError> {
        Ok(self
            .inner
            .read()
            .kv
            .iter()
            .filter(|((c, _), _)| c == collection)
            .map(|(_, v)| v.clone())
            .collect())
    }

    fn delete_kv(&self, collection: &str, id: &str) -> Result<bool, StorageError> {
        Ok(self
            .inner
            .write()
            .kv
            .remove(&(collection.to_owned(), id.to_owned()))
            .is_some())
    }
}

impl BlobStore for MemoryCatalog {
    fn put(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), StorageError> {
        self.inner
            .write()
            .blobs
            .insert((namespace.to_owned(), key.to_owned()), value.to_vec());
        Ok(())
    }

    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .inner
            .read()
            .blobs
            .get(&(namespace.to_owned(), key.to_owned()))
            .cloned())
    }

    fn delete(&self, namespace: &str, key: &str) -> Result<(), StorageError> {
        self.inner
            .write()
            .blobs
            .remove(&(namespace.to_owned(), key.to_owned()));
        Ok(())
    }
}
