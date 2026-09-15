//! Built-in configurators: `executor.<kind>.<name>` and `executor.coreTools.*`.

use std::collections::BTreeMap;

use executor_core::{
    AuthTemplateSlug, ConnectionInput, ConnectionName, ConnectionRef, IntegrationSlug, Owner,
    PluginId, PolicyAction, PolicyId, PolicyPattern, RegisterIntegration, SecretRef, Tool,
    ToolAnnotations, ToolName, ToolPolicy, ToolResult, tool_address,
};
use serde_json::{Value, json};

use crate::Inner;

const OPENAPI_ADD: &str = "executor.openapi.addSpec";
const OPENAPI_ADD_ALIAS: &str = "executor.openapi.addIntegration";
const GRAPHQL_ADD: &str = "executor.graphql.addIntegration";
const MCP_ADD: &str = "executor.mcp.addServer";

impl Inner {
    pub(crate) fn install_static_tools(&self) {
        let mut tools = BTreeMap::new();
        push_core_tools(&mut tools);
        self.push_plugin_static(&mut tools);
        *self.static_tools.write() = tools;
    }

    fn push_plugin_static(&self, tools: &mut BTreeMap<String, Tool>) {
        if self.plugins.contains_key("openapi") {
            push_static(
                tools,
                OPENAPI_ADD,
                "Add an OpenAPI / Swagger / Google Discovery integration. Pass `spec` (document or {url}) and `slug`.",
                add_spec_schema(),
                true,
            );
            push_static(
                tools,
                OPENAPI_ADD_ALIAS,
                "Alias of executor.openapi.addSpec.",
                add_spec_schema(),
                true,
            );
        }
        if self.plugins.contains_key("graphql") {
            push_static(
                tools,
                GRAPHQL_ADD,
                "Add a GraphQL integration (`endpoint`, `slug`, optional `schema`).",
                json!({
                    "type":"object",
                    "required":["endpoint","slug"],
                    "properties":{
                        "endpoint":{"type":"string"},
                        "slug":{"type":"string"},
                        "name":{"type":"string"},
                        "description":{"type":"string"},
                        "schema":{}
                    }
                }),
                true,
            );
        }
        if self.plugins.contains_key("mcp") {
            push_static(
                tools,
                MCP_ADD,
                "Add an MCP server integration (`url` for HTTP, or `command` for stdio).",
                json!({
                    "type":"object",
                    "required":["slug"],
                    "properties":{
                        "slug":{"type":"string"},
                        "name":{"type":"string"},
                        "description":{"type":"string"},
                        "url":{"type":"string"},
                        "command":{"type":"string"},
                        "args":{"type":"array","items":{"type":"string"}},
                        "transport":{"type":"string","enum":["http","stdio","auto"]}
                    }
                }),
                true,
            );
        }
    }

    pub(crate) async fn invoke_static(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        match name {
            "executor.coreTools.integrations.list" => self.st_integrations_list(),
            "executor.coreTools.integrations.remove" => self.st_integrations_remove(args),
            "executor.coreTools.connections.list" => self.st_connections_list(args),
            "executor.coreTools.connections.create" => self.st_connections_create(args).await,
            "executor.coreTools.connections.remove" => self.st_connections_remove(args),
            "executor.coreTools.connections.refresh" => self.st_connections_refresh(args).await,
            "executor.coreTools.policies.list" => self.st_policies_list(),
            "executor.coreTools.policies.create" => self.st_policies_create(args),
            "executor.coreTools.policies.remove" => self.st_policies_remove(args),
            OPENAPI_ADD | OPENAPI_ADD_ALIAS => self.st_openapi_add(args),
            GRAPHQL_ADD => self.st_graphql_add(args),
            MCP_ADD => self.st_mcp_add(args),
            other => Err(executor_core::ExecutorError::tool_not_found(
                other.to_owned(),
                Vec::new(),
            )),
        }
    }

    fn st_integrations_list(&self) -> Result<ToolResult, executor_core::ExecutorError> {
        let rows: Vec<Value> = self
            .store
            .list_integrations()?
            .into_iter()
            .map(|r| {
                json!({
                    "slug": r.integration.slug.as_str(),
                    "name": r.integration.name,
                    "description": r.integration.description,
                    "kind": r.integration.kind.as_str(),
                    "canRemove": r.integration.can_remove,
                    "canRefresh": r.integration.can_refresh,
                })
            })
            .collect();
        Ok(ToolResult::ok(json!({ "integrations": rows })))
    }

    fn st_integrations_remove(
        &self,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let slug = IntegrationSlug::new(req_str(args, "slug")?)?;
        let Some(row) = self.store.get_integration(&slug)? else {
            return Err(executor_core::ExecutorError::IntegrationNotFound(slug));
        };
        if !row.integration.can_remove {
            return Err(executor_core::ExecutorError::RemovalNotAllowed(slug));
        }
        let existed = self.store.remove_integration(&slug)?;
        Ok(ToolResult::ok(
            json!({ "removed": existed, "slug": slug.as_str() }),
        ))
    }

    fn st_connections_list(
        &self,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let integration = match args.get("integration").and_then(Value::as_str) {
            Some(s) => Some(IntegrationSlug::new(s)?),
            None => None,
        };
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse);
        let rows: Vec<Value> = self
            .store
            .list_connections(integration.as_ref(), owner)?
            .into_iter()
            .map(|c| {
                json!({
                    "owner": c.owner.as_str(),
                    "name": c.name.as_str(),
                    "integration": c.integration.as_str(),
                    "template": c.template.as_str(),
                    "address": c.address.to_string(),
                })
            })
            .collect();
        Ok(ToolResult::ok(json!({ "connections": rows })))
    }

    async fn st_connections_create(
        &self,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let mut values = BTreeMap::new();
        if let Some(obj) = args.get("values").and_then(Value::as_object) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    values.insert(k.clone(), s.to_owned());
                }
            }
        }
        let mut refs = executor_core::CredentialMap::new();
        if let Some(obj) = args.get("refs").and_then(Value::as_object) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    refs.insert(
                        k.clone(),
                        SecretRef::parse(s).map_err(executor_core::ExecutorError::InvalidArgs)?,
                    );
                }
            }
        }
        let template = args
            .get("template")
            .and_then(Value::as_str)
            .map(AuthTemplateSlug::new)
            .transpose()?
            .unwrap_or_else(AuthTemplateSlug::none);
        let conn = self
            .create_connection(ConnectionInput {
                owner,
                name: ConnectionName::new(req_str(args, "name")?)?,
                integration: IntegrationSlug::new(req_str(args, "integration")?)?,
                template,
                identity_label: None,
                description: None,
                values,
                refs,
            })
            .await?;
        Ok(ToolResult::ok(json!({
            "owner": conn.owner.as_str(),
            "name": conn.name.as_str(),
            "integration": conn.integration.as_str(),
            "address": conn.address.to_string(),
        })))
    }

    fn st_connections_remove(
        &self,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let id = conn_ref(args)?;
        let existed = self.store.remove_connection(&id)?;
        Ok(ToolResult::ok(json!({ "removed": existed })))
    }

    async fn st_connections_refresh(
        &self,
        args: &Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let id = conn_ref(args)?;
        let tools = self.refresh_connection(&id).await?;
        Ok(ToolResult::ok(json!({ "toolCount": tools.len() })))
    }

    fn st_policies_list(&self) -> Result<ToolResult, executor_core::ExecutorError> {
        let rows: Vec<Value> = self
            .store
            .list_policies()?
            .into_iter()
            .map(|p| {
                json!({
                    "id": p.id.as_str(),
                    "owner": p.owner.as_str(),
                    "pattern": p.pattern.as_str(),
                    "action": p.action,
                    "position": p.position,
                })
            })
            .collect();
        Ok(ToolResult::ok(json!({ "policies": rows })))
    }

    fn st_policies_create(&self, args: &Value) -> Result<ToolResult, executor_core::ExecutorError> {
        let pattern_raw = req_str(args, "pattern")?;
        let pattern = PolicyPattern::new(pattern_raw)
            .map_err(executor_core::ExecutorError::InvalidPattern)?;
        let action = parse_action(req_str(args, "action")?)?;
        let owner = args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org);
        let existing = self.store.list_policies()?;
        let owner_rows: Vec<(String, String, String)> = existing
            .iter()
            .filter(|p| p.owner == owner)
            .map(|p| {
                (
                    p.pattern.as_str().to_owned(),
                    p.position.clone(),
                    p.id.as_str().to_owned(),
                )
            })
            .collect();
        let position = executor_core::position_for_new_pattern(pattern.as_str(), &owner_rows);
        let id = PolicyId::new(format!("pol_{}", executor_core::unix_now_ms()))?;
        let row = ToolPolicy {
            id: id.clone(),
            owner,
            pattern,
            action,
            position,
        };
        self.store.put_policy(row)?;
        Ok(ToolResult::ok(json!({ "id": id.as_str() })))
    }

    fn st_policies_remove(&self, args: &Value) -> Result<ToolResult, executor_core::ExecutorError> {
        let id = PolicyId::new(req_str(args, "id")?)?;
        let existed = self.store.remove_policy(&id)?;
        Ok(ToolResult::ok(json!({ "removed": existed })))
    }

    fn st_openapi_add(&self, args: &Value) -> Result<ToolResult, executor_core::ExecutorError> {
        self.register_kind(args, PluginId::openapi(), openapi_config(args)?)
    }

    fn st_graphql_add(&self, args: &Value) -> Result<ToolResult, executor_core::ExecutorError> {
        let endpoint = req_str(args, "endpoint")?;
        let mut config = serde_json::Map::new();
        config.insert("endpoint".into(), Value::String(endpoint.to_owned()));
        if let Some(schema) = args.get("schema") {
            config.insert("schema".into(), schema.clone());
        }
        copy_oauth_config(args, &mut config);
        self.register_kind(args, PluginId::graphql(), Value::Object(config))
    }

    fn st_mcp_add(&self, args: &Value) -> Result<ToolResult, executor_core::ExecutorError> {
        let mut config = serde_json::Map::new();
        if let Some(url) = args.get("url") {
            config.insert("url".into(), url.clone());
        }
        if let Some(command) = args.get("command") {
            config.insert("command".into(), command.clone());
        }
        if let Some(cmd_args) = args.get("args") {
            config.insert("args".into(), cmd_args.clone());
        }
        let transport = args
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if config.contains_key("command") {
                    "stdio"
                } else {
                    "http"
                }
            });
        config.insert("transport".into(), Value::String(transport.to_owned()));
        self.register_kind(args, PluginId::mcp(), Value::Object(config))
    }

    fn register_kind(
        &self,
        args: &Value,
        kind: PluginId,
        config: Value,
    ) -> Result<ToolResult, executor_core::ExecutorError> {
        let slug = IntegrationSlug::new(req_str(args, "slug")?)?;
        if self.store.get_integration(&slug)?.is_some() {
            return Err(executor_core::ExecutorError::Conflict(format!(
                "integration {slug} already exists"
            )));
        }
        let bytes = serde_json::to_vec(&config).unwrap_or_default().len();
        if bytes > self.limits.max_spec_bytes {
            return Err(executor_core::ExecutorError::LimitExceeded(
                "max_spec_bytes".into(),
            ));
        }
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let description = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let integration = self.register_integration(
            RegisterIntegration {
                slug,
                name,
                description,
                config,
                can_remove: true,
                can_refresh: true,
            },
            kind,
        )?;
        Ok(ToolResult::ok(json!({
            "slug": integration.slug.as_str(),
            "name": integration.name,
            "kind": integration.kind.as_str(),
        })))
    }
}

fn push_core_tools(tools: &mut BTreeMap<String, Tool>) {
    push_integration_tools(tools);
    push_connection_tools(tools);
    push_policy_tools(tools);
}

fn push_integration_tools(tools: &mut BTreeMap<String, Tool>) {
    push_static(
        tools,
        "executor.coreTools.integrations.list",
        "List catalog integrations.",
        json!({"type":"object","properties":{}}),
        false,
    );
    push_static(
        tools,
        "executor.coreTools.integrations.remove",
        "Remove a removable integration.",
        json!({"type":"object","required":["slug"],"properties":{"slug":{"type":"string"}}}),
        true,
    );
}

fn push_connection_tools(tools: &mut BTreeMap<String, Tool>) {
    push_static(
        tools,
        "executor.coreTools.connections.list",
        "List saved connections.",
        json!({
            "type":"object",
            "properties":{
                "integration":{"type":"string"},
                "owner":{"type":"string","enum":["org","user"]}
            }
        }),
        false,
    );
    push_static(
        tools,
        "executor.coreTools.connections.create",
        "Create a connection (credential). Values are stored as secrets.",
        json!({
            "type":"object",
            "required":["name","integration"],
            "properties":{
                "owner":{"type":"string","enum":["org","user"]},
                "name":{"type":"string"},
                "integration":{"type":"string"},
                "template":{"type":"string"},
                "values":{"type":"object"},
                "refs":{"type":"object"}
            }
        }),
        true,
    );
    push_static(
        tools,
        "executor.coreTools.connections.remove",
        "Remove a connection.",
        json!({
            "type":"object",
            "required":["integration","name"],
            "properties":{
                "owner":{"type":"string","enum":["org","user"]},
                "integration":{"type":"string"},
                "name":{"type":"string"}
            }
        }),
        true,
    );
    push_static(
        tools,
        "executor.coreTools.connections.refresh",
        "Refresh tools for a connection.",
        json!({
            "type":"object",
            "required":["integration","name"],
            "properties":{
                "owner":{"type":"string","enum":["org","user"]},
                "integration":{"type":"string"},
                "name":{"type":"string"}
            }
        }),
        false,
    );
}

fn push_policy_tools(tools: &mut BTreeMap<String, Tool>) {
    push_static(
        tools,
        "executor.coreTools.policies.list",
        "List tool policies.",
        json!({"type":"object","properties":{}}),
        false,
    );
    push_static(
        tools,
        "executor.coreTools.policies.create",
        "Create a tool policy (org is outer-wins).",
        json!({
            "type":"object",
            "required":["pattern","action"],
            "properties":{
                "owner":{"type":"string","enum":["org","user"]},
                "pattern":{"type":"string"},
                "action":{"type":"string","enum":["approve","require_approval","block"]}
            }
        }),
        true,
    );
    push_static(
        tools,
        "executor.coreTools.policies.remove",
        "Remove a policy by id.",
        json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
        true,
    );
}

fn add_spec_schema() -> Value {
    json!({
        "type":"object",
        "required":["slug","spec"],
        "properties":{
            "slug":{"type":"string"},
            "name":{"type":"string"},
            "description":{"type":"string"},
            "baseUrl":{"type":"string"},
            "tag":{"type":"string"},
            "spec":{}
        }
    })
}

fn openapi_config(args: &Value) -> Result<Value, executor_core::ExecutorError> {
    let spec = args
        .get("spec")
        .cloned()
        .ok_or_else(|| executor_core::ExecutorError::InvalidArgs("missing spec".into()))?;
    let mut config = serde_json::Map::new();
    match spec {
        Value::String(text) => {
            config.insert("spec".into(), Value::String(text));
        }
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                config.insert("specUrl".into(), Value::String(url.to_owned()));
            } else if let Some(text) = map.get("blob").and_then(Value::as_str).or_else(|| {
                map.get("value")
                    .and_then(Value::as_str)
                    .or_else(|| map.get("text").and_then(Value::as_str))
            }) {
                config.insert("spec".into(), Value::String(text.to_owned()));
            } else {
                config.insert("spec".into(), Value::Object(map));
            }
        }
        other => {
            config.insert("spec".into(), other);
        }
    }
    if let Some(base) = args.get("baseUrl").and_then(Value::as_str) {
        config.insert("baseUrl".into(), Value::String(base.to_owned()));
    }
    if let Some(tag) = args.get("tag").and_then(Value::as_str) {
        config.insert("tag".into(), Value::String(tag.to_owned()));
    }
    copy_oauth_config(args, &mut config);
    Ok(Value::Object(config))
}

fn push_static(
    tools: &mut BTreeMap<String, Tool>,
    fqid: &str,
    description: &str,
    schema: Value,
    requires_approval: bool,
) {
    let Ok(name) = ToolName::new(fqid) else {
        return;
    };
    let Ok(integration) = IntegrationSlug::new("executor") else {
        return;
    };
    let Ok(connection) = ConnectionName::new("core") else {
        return;
    };
    let tool = Tool {
        address: tool_address(
            Owner::Org,
            integration.clone(),
            connection.clone(),
            name.clone(),
        ),
        owner: Owner::Org,
        integration,
        connection,
        name,
        plugin_id: PluginId::core_tools(),
        description: description.to_owned(),
        input_schema: Some(schema),
        output_schema: None,
        annotations: Some(ToolAnnotations {
            requires_approval: Some(requires_approval),
            approval_description: requires_approval.then(|| description.to_owned()),
            may_elicit: None,
        }),
        static_tool: true,
        plugin_meta: None,
    };
    tools.insert(fqid.to_owned(), tool);
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, executor_core::ExecutorError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| executor_core::ExecutorError::InvalidArgs(format!("missing {key}")))
}

fn conn_ref(args: &Value) -> Result<ConnectionRef, executor_core::ExecutorError> {
    Ok(ConnectionRef {
        owner: args
            .get("owner")
            .and_then(Value::as_str)
            .and_then(Owner::parse)
            .unwrap_or(Owner::Org),
        name: ConnectionName::new(req_str(args, "name")?)?,
        integration: IntegrationSlug::new(req_str(args, "integration")?)?,
    })
}

fn parse_action(raw: &str) -> Result<PolicyAction, executor_core::ExecutorError> {
    match raw {
        "approve" => Ok(PolicyAction::Approve),
        "require_approval" => Ok(PolicyAction::RequireApproval),
        "block" => Ok(PolicyAction::Block),
        other => Err(executor_core::ExecutorError::InvalidArgs(format!(
            "unknown action {other}"
        ))),
    }
}

fn copy_oauth_config(args: &Value, config: &mut serde_json::Map<String, Value>) {
    for key in ["authorizationUrl", "tokenUrl"] {
        if let Some(url) = args.get(key).and_then(Value::as_str) {
            config.insert(key.to_owned(), Value::String(url.to_owned()));
        }
    }
    if let Some(scopes) = args.get("scopes") {
        config.insert("scopes".into(), scopes.clone());
    }
}
