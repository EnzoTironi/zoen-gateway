//! Catalog mutations: integrations, connections, tool refresh.

use executor_core::{
    Connection, ConnectionInput, ConnectionRef, CredentialMap, ExecutorError, Integration,
    IntegrationRecord, PluginId, ProviderKey, RegisterIntegration, ResolveToolsCtx, Tool, ToolDef,
    connection_address, tool_address,
};
use tracing::instrument;

use crate::Inner;

impl Inner {
    #[instrument(skip(self, input), fields(slug = %input.slug))]
    pub(crate) fn register_integration(
        &self,
        input: RegisterIntegration,
        kind: PluginId,
    ) -> Result<Integration, ExecutorError> {
        let plugin = self
            .plugins
            .get(kind.as_str())
            .ok_or_else(|| ExecutorError::PluginNotLoaded(kind.to_string()))?;
        let auth_methods = plugin.describe_auth(&input.config);
        let name = input
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| input.slug.to_string());
        let integration = Integration {
            slug: input.slug.clone(),
            name,
            description: input.description,
            kind,
            can_remove: input.can_remove,
            can_refresh: input.can_refresh,
            auth_methods,
            display_url: None,
        };
        self.store.put_integration(IntegrationRecord {
            integration: integration.clone(),
            config: input.config,
        })?;
        Ok(integration)
    }

    pub(crate) async fn create_connection(
        &self,
        mut input: ConnectionInput,
    ) -> Result<Connection, ExecutorError> {
        self.store
            .get_integration(&input.integration)?
            .ok_or_else(|| ExecutorError::IntegrationNotFound(input.integration.clone()))?;
        let mut secrets = CredentialMap::new();
        let pasted = std::mem::take(&mut input.values);
        for (var, value) in pasted {
            secrets.insert(var, self.secrets.store_default(&value)?);
        }
        secrets.append(&mut input.refs);
        let conn = Connection {
            owner: input.owner,
            name: input.name.clone(),
            integration: input.integration.clone(),
            template: input.template,
            provider: ProviderKey::default_store(),
            address: connection_address(input.owner, &input.integration, &input.name),
            identity_label: input.identity_label,
            description: input.description,
            secrets,
            last_health: None,
        };
        let id = ConnectionRef {
            owner: conn.owner,
            name: conn.name.clone(),
            integration: conn.integration.clone(),
        };
        if self.store.get_connection(&id)?.is_some() {
            return Err(ExecutorError::Conflict(format!(
                "connection {} already exists",
                conn.address
            )));
        }
        self.store.put_connection(conn.clone())?;
        self.refresh_connection(&id).await?;
        Ok(conn)
    }

    pub(crate) async fn refresh_connection(
        &self,
        id: &ConnectionRef,
    ) -> Result<Vec<Tool>, ExecutorError> {
        let conn = self
            .store
            .get_connection(id)?
            .ok_or_else(|| ExecutorError::ConnectionNotFound(id.as_key()))?;
        let record = self
            .store
            .get_integration(&id.integration)?
            .ok_or_else(|| ExecutorError::IntegrationNotFound(id.integration.clone()))?;
        let plugin = self
            .plugins
            .get(record.integration.kind.as_str())
            .ok_or_else(|| ExecutorError::PluginNotLoaded(record.integration.kind.to_string()))?;
        let values = self.resolve_secrets(&conn)?;
        let catalog_len = self.store.list_tools()?.len();
        if catalog_len >= self.limits.max_catalog_tools {
            return Err(ExecutorError::LimitExceeded("max_catalog_tools".to_owned()));
        }
        let ctx = ResolveToolsCtx {
            integration: &record.integration,
            config: &record.config,
            connection: id,
            values: &values,
            timeout: self.limits.http_timeout,
            max_tools: self.limits.max_tools_per_connection,
            max_spec_bytes: self.limits.max_spec_bytes,
        };
        let resolved = plugin
            .resolve_tools(ctx)
            .await
            .map_err(|e| ExecutorError::Plugin(e.0))?;
        if resolved.incomplete {
            tracing::warn!(
                reason = resolved.incomplete_reason.as_deref(),
                "incomplete tool listing; keeping prior catalog"
            );
            return self.tools_for_connection(id);
        }
        if resolved.tools.len() > self.limits.max_tools_per_connection {
            return Err(ExecutorError::LimitExceeded(format!(
                "connection produced {} tools (max {})",
                resolved.tools.len(),
                self.limits.max_tools_per_connection
            )));
        }
        let tools = stamp_tools(&record.integration.kind, id, resolved.tools)?;
        self.store.replace_tools(id, tools.clone())?;
        Ok(tools)
    }

    pub(crate) fn tools_for_connection(
        &self,
        id: &ConnectionRef,
    ) -> Result<Vec<Tool>, ExecutorError> {
        Ok(self
            .store
            .list_tools()?
            .into_iter()
            .filter(|t| tool_on_connection(t, id))
            .collect())
    }

    pub(crate) fn resolve_secrets(
        &self,
        conn: &Connection,
    ) -> Result<executor_core::CredentialMapValues, ExecutorError> {
        let mut out = executor_core::CredentialMapValues::new();
        for (k, r) in &conn.secrets {
            out.insert(k.clone(), self.secrets.resolve(r)?);
        }
        Ok(out)
    }
}

#[allow(clippy::suspicious_operation_groupings)] // `ConnectionRef.name` is the connection name, not `tool.name`.
fn tool_on_connection(tool: &Tool, id: &ConnectionRef) -> bool {
    tool.owner == id.owner
        && tool.integration.as_str() == id.integration.as_str()
        && tool.connection.as_str() == id.name.as_str()
}

fn stamp_tools(
    plugin: &PluginId,
    id: &ConnectionRef,
    defs: Vec<ToolDef>,
) -> Result<Vec<Tool>, ExecutorError> {
    defs.into_iter()
        .map(|def| {
            Ok(Tool {
                address: tool_address(
                    id.owner,
                    id.integration.clone(),
                    id.name.clone(),
                    def.name.clone(),
                ),
                owner: id.owner,
                integration: id.integration.clone(),
                connection: id.name.clone(),
                name: def.name,
                plugin_id: plugin.clone(),
                description: def.description,
                input_schema: def.input_schema,
                output_schema: def.output_schema,
                annotations: def.annotations,
                static_tool: false,
                plugin_meta: def.plugin_meta,
            })
        })
        .collect()
}
