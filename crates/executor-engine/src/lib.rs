//! Bounded execute runtime: catalog, policy, pause/resume, static core-tools.
//!
//! Catalog IO never holds a lock across plugin work. In-flight executes are
//! capped by a semaphore; overflow is [`ExecutorError::Overloaded`], not a
//! queue.

#![allow(clippy::result_large_err)] // ExecutorError is the domain error; boxing hides fields.
#![allow(clippy::module_name_repetitions)] // `Executor` vocabulary (`ExecuteOptions` lives in core).

mod agent_catalog;
mod catalog;
mod code;
mod ema;
mod execute;
mod lookup;
mod oauth;
mod static_tools;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use executor_core::{
    CatalogStore, Connection, ConnectionInput, ConnectionRef, ExecuteOptions, ExecutionId,
    ExecutorError, Integration, IntegrationPlugin, IntegrationSlug, Limits, MemoryCatalog, Metrics,
    NoopMetrics, Outcome, PluginId, RegisterIntegration, ResumeAction, Tool, ToolListFilter,
};
use executor_secrets::{MemorySecrets, SecretResolver};
use parking_lot::RwLock;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub use ema::{
    AuthorizationServerMetadata, EmaMintInput, EnterpriseManagedGrant,
    discover_authorization_server_metadata, mint_enterprise_managed_access_token,
    resource_metadata_urls, run_enterprise_managed_authorization, try_fill_token,
};
pub use oauth::{
    DcrClient, authorization_request, client_credentials, exchange_code, pkce_pair, register_client,
};

/// Shared runtime state. Modules implement methods on this type.
pub(crate) struct Inner {
    pub(crate) store: Arc<dyn CatalogStore>,
    pub(crate) secrets: Arc<dyn SecretResolver>,
    pub(crate) plugins: HashMap<String, Arc<dyn IntegrationPlugin>>,
    pub(crate) limits: Limits,
    pub(crate) metrics: Arc<dyn Metrics>,
    pub(crate) semaphore: Arc<Semaphore>,
    pub(crate) static_tools: RwLock<BTreeMap<String, Tool>>,
    pub(crate) cancel: CancellationToken,
    pub(crate) in_flight: Arc<AtomicU64>,
}

/// Cloneable executor handle. Cheap: `Arc` interior.
#[derive(Clone)]
pub struct Executor {
    pub(crate) inner: Arc<Inner>,
}

/// Builder for [`Executor`]. Missing store/secrets default to in-memory.
pub struct ExecutorBuilder {
    store: Option<Arc<dyn CatalogStore>>,
    secrets: Option<Arc<dyn SecretResolver>>,
    plugins: HashMap<String, Arc<dyn IntegrationPlugin>>,
    limits: Limits,
    metrics: Arc<dyn Metrics>,
    cancel: CancellationToken,
}

impl Default for ExecutorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutorBuilder {
    /// Empty builder with production limits and no-op metrics.
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: None,
            secrets: None,
            plugins: HashMap::new(),
            limits: Limits::production(),
            metrics: Arc::new(NoopMetrics),
            cancel: CancellationToken::new(),
        }
    }

    /// Catalog backend.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn CatalogStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Secret resolvers.
    #[must_use]
    pub fn secrets(mut self, secrets: Arc<dyn SecretResolver>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Register a first-party or third-party plugin (last writer wins on id).
    #[must_use]
    pub fn plugin(mut self, plugin: Arc<dyn IntegrationPlugin>) -> Self {
        self.plugins.insert(plugin.id().to_string(), plugin);
        self
    }

    /// Resource bounds.
    #[must_use]
    pub const fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Metrics sink.
    #[must_use]
    pub fn metrics(mut self, metrics: Arc<dyn Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Process-wide cancellation (daemon shutdown).
    #[must_use]
    pub fn cancellation(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Assemble the runtime and install static tools for loaded plugins.
    #[must_use]
    pub fn build(self) -> Executor {
        let permits = usize::try_from(self.limits.max_in_flight).unwrap_or(usize::MAX);
        let inner = Inner {
            store: self.store.unwrap_or_else(|| Arc::new(MemoryCatalog::new())),
            secrets: self
                .secrets
                .unwrap_or_else(|| Arc::new(MemorySecrets::new())),
            plugins: self.plugins,
            limits: self.limits,
            metrics: self.metrics,
            semaphore: Arc::new(Semaphore::new(permits.max(1))),
            static_tools: RwLock::new(BTreeMap::new()),
            cancel: self.cancel,
            in_flight: Arc::new(AtomicU64::new(0)),
        };
        let exec = Executor {
            inner: Arc::new(inner),
        };
        exec.inner.install_static_tools();
        exec
    }
}

impl Executor {
    /// Start a builder.
    #[must_use]
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder::new()
    }

    /// Process cancellation token. Cancelled by [`Self::shutdown`].
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancel.clone()
    }

    /// Abort in-flight work. Idempotent.
    pub fn shutdown(&self) {
        self.inner.cancel.cancel();
    }

    /// Configured limits.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Metrics sink (for `/metrics` scrape).
    #[must_use]
    pub fn metrics(&self) -> Arc<dyn Metrics> {
        Arc::clone(&self.inner.metrics)
    }

    /// Register or replace an integration.
    ///
    /// # Errors
    ///
    /// Unknown plugin, catalog write, or identifier errors.
    pub fn register_integration(
        &self,
        input: RegisterIntegration,
        kind: PluginId,
    ) -> Result<Integration, ExecutorError> {
        self.inner.register_integration(input, kind)
    }

    /// Create a connection and refresh its tools.
    ///
    /// # Errors
    ///
    /// Missing integration, conflict, secret store, plugin, or limits.
    pub async fn create_connection(
        &self,
        input: ConnectionInput,
    ) -> Result<Connection, ExecutorError> {
        self.inner.create_connection(input).await
    }

    /// Re-resolve tools for one connection.
    ///
    /// # Errors
    ///
    /// Missing connection/plugin, or limits.
    pub async fn refresh_connection(&self, id: &ConnectionRef) -> Result<Vec<Tool>, ExecutorError> {
        self.inner.refresh_connection(id).await
    }

    /// Invoke a static or catalog tool. Bounded, timed, cancellable.
    ///
    /// # Errors
    ///
    /// Overload, timeout, cancel, policy, missing tool, schema, plugin, storage.
    pub async fn execute(
        &self,
        path: &str,
        args: serde_json::Value,
        opts: ExecuteOptions,
    ) -> Result<Outcome, ExecutorError> {
        self.inner
            .execute(path, args, opts, self.inner.cancel.child_token())
            .await
    }

    /// [`Self::execute`] with an extra cancel token (combined with process shutdown).
    ///
    /// # Errors
    ///
    /// Same as [`Self::execute`].
    pub async fn execute_with_cancel(
        &self,
        path: &str,
        args: serde_json::Value,
        opts: ExecuteOptions,
        cancel: CancellationToken,
    ) -> Result<Outcome, ExecutorError> {
        let linked = self.inner.cancel.child_token();
        let cancel_task = cancel.clone();
        let linked_task = linked.clone();
        tokio::spawn(async move {
            cancel_task.cancelled().await;
            linked_task.cancel();
        });
        self.inner.execute(path, args, opts, linked).await
    }

    /// Resume a paused execution (single-use).
    ///
    /// # Errors
    ///
    /// Missing/consumed execution, not paused, invoke failures.
    pub async fn resume(
        &self,
        id: &ExecutionId,
        action: ResumeAction,
    ) -> Result<Outcome, ExecutorError> {
        self.inner
            .resume(id, action, self.inner.cancel.child_token())
            .await
    }

    /// Agent-facing tool list (blocked tools omitted unless the filter says otherwise).
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn list_tools(&self, filter: &ToolListFilter) -> Result<Vec<Tool>, ExecutorError> {
        self.inner.list_filtered(filter)
    }

    /// Integrations in the catalog.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn list_integrations(&self) -> Result<Vec<Integration>, ExecutorError> {
        Ok(self
            .inner
            .store
            .list_integrations()?
            .into_iter()
            .map(|r| r.integration)
            .collect())
    }

    /// Remove an integration if `can_remove`.
    ///
    /// # Errors
    ///
    /// Missing, not removable, storage.
    pub fn remove_integration(&self, slug: &IntegrationSlug) -> Result<bool, ExecutorError> {
        let Some(row) = self.inner.store.get_integration(slug)? else {
            return Err(ExecutorError::IntegrationNotFound(slug.clone()));
        };
        if !row.integration.can_remove {
            return Err(ExecutorError::RemovalNotAllowed(slug.clone()));
        }
        Ok(self.inner.store.remove_integration(slug)?)
    }

    /// Connections, optionally filtered.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn list_connections(
        &self,
        integration: Option<&IntegrationSlug>,
        owner: Option<executor_core::Owner>,
    ) -> Result<Vec<Connection>, ExecutorError> {
        Ok(self.inner.store.list_connections(integration, owner)?)
    }

    /// Drop a connection and its dynamic tools.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn remove_connection(&self, id: &ConnectionRef) -> Result<bool, ExecutorError> {
        Ok(self.inner.store.remove_connection(id)?)
    }

    /// Resolve a CLI path for inspect/describe.
    ///
    /// # Errors
    ///
    /// Not found.
    pub fn describe(&self, path: &str) -> Result<Tool, ExecutorError> {
        Ok(self.inner.resolve_path(path)?.tool().clone())
    }

    /// Loaded plugin ids.
    #[must_use]
    pub fn plugin_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.plugins.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Run each plugin's `detect` against `url`.
    #[must_use]
    pub fn detect(&self, url: &str) -> Vec<executor_core::Detection> {
        let mut hits: Vec<executor_core::Detection> = self
            .inner
            .plugins
            .values()
            .filter_map(|p| p.detect(url))
            .collect();
        hits.sort_by(|a, b| a.kind.as_str().cmp(b.kind.as_str()));
        hits
    }

    /// Policies in the catalog.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn list_policies(&self) -> Result<Vec<executor_core::ToolPolicy>, ExecutorError> {
        Ok(self.inner.store.list_policies()?)
    }

    /// Load a persisted execution (paused or completed).
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn get_execution(
        &self,
        id: &ExecutionId,
    ) -> Result<Option<executor_core::ExecutionState>, ExecutorError> {
        Ok(self.inner.store.get_execution(id)?)
    }

    /// Load a toolkit by slug.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn toolkit_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<executor_core::Toolkit>, ExecutorError> {
        let Some(body) = self.inner.store.get_kv(executor_core::KV_TOOLKITS, slug)? else {
            return Ok(None);
        };
        Ok(serde_json::from_value(body).ok())
    }

    /// KV put (toolkits / oauth clients).
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn put_kv(
        &self,
        collection: &str,
        id: &str,
        body: serde_json::Value,
    ) -> Result<(), ExecutorError> {
        Ok(self.inner.store.put_kv(collection, id, body)?)
    }

    /// KV get.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn get_kv(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, ExecutorError> {
        Ok(self.inner.store.get_kv(collection, id)?)
    }

    /// KV list.
    ///
    /// # Errors
    ///
    /// Storage.
    pub fn list_kv(&self, collection: &str) -> Result<Vec<serde_json::Value>, ExecutorError> {
        Ok(self.inner.store.list_kv(collection)?)
    }
}

/// Run a catalog op on a blocking thread so SQLite never parks a tokio worker.
pub(crate) async fn on_store<T, F>(store: Arc<dyn CatalogStore>, f: F) -> Result<T, ExecutorError>
where
    T: Send + 'static,
    F: FnOnce(&dyn CatalogStore) -> Result<T, executor_core::StorageError> + Send + 'static,
{
    match tokio::task::spawn_blocking(move || f(&*store)).await {
        Ok(result) => result.map_err(ExecutorError::from),
        Err(join) if join.is_cancelled() => Err(ExecutorError::Cancelled),
        Err(join) => Err(ExecutorError::Storage(executor_core::StorageError::new(
            join.to_string(),
        ))),
    }
}

#[cfg(test)]
mod tests;
