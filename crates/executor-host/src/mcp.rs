//! JSON-RPC MCP: Streamable HTTP + stdio. Default tools are `execute` / `skills` / `resume`.

use std::collections::HashMap;
use std::sync::Arc;

use executor_catalog::{CallInput, CatalogService};
use executor_core::{
    ExecuteOptions, ExecutionId, LOCAL_SUBJECT, PersistChoice, ResumeAction, ResumeRequest,
    SearchArgs, Toolkit,
};
use executor_engine::Executor;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::instrument;

use crate::skills::{execute_description, skills_result};

/// Codemode vs JSON search/invoke surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum McpMode {
    /// `execute` + `skills` + `resume` (default).
    #[default]
    Code,
    /// `integrations` + `search` + `invoke` + `skills`.
    Passthrough,
}

impl McpMode {
    /// Parse `mode` query (`code` / `passthrough`).
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("code") {
            "passthrough" => Self::Passthrough,
            _ => Self::Code,
        }
    }
}

/// How `resume` is exposed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ElicitationMode {
    /// Model cannot choose accept/decline; resume only takes `executionId`.
    #[default]
    Browser,
    /// Model may pass `action` / `content`.
    Model,
}

impl ElicitationMode {
    /// Parse `elicitation_mode` query.
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("browser") {
            "model" => Self::Model,
            _ => Self::Browser,
        }
    }
}

/// Per-session MCP options.
#[derive(Clone, Debug, Default)]
pub struct McpOptions {
    /// Surface.
    pub mode: McpMode,
    /// Resume schema.
    pub elicitation: ElicitationMode,
    /// Opt-in `search_<integration>` tools.
    pub search_tools: bool,
    /// Optional toolkit scope.
    pub toolkit: Option<Toolkit>,
}

/// In-memory Streamable HTTP sessions.
#[derive(Default)]
pub struct McpHub {
    sessions: Mutex<HashMap<String, McpOptions>>,
}

impl McpHub {
    /// Empty hub.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a session and return its id.
    #[must_use]
    pub fn create(&self, opts: McpOptions) -> String {
        let id = new_session_id();
        self.sessions.lock().insert(id.clone(), opts);
        id
    }

    /// Look up a session.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<McpOptions> {
        self.sessions.lock().get(id).cloned()
    }

    /// Drop a session.
    pub fn delete(&self, id: &str) -> bool {
        self.sessions.lock().remove(id).is_some()
    }
}

fn new_session_id() -> String {
    let mut buf = [0_u8; 16];
    let _ = getrandom::getrandom(&mut buf);
    hex::encode(buf)
}

/// Handle a single JSON-RPC object (or a notification) with default code-mode options.
pub async fn handle_jsonrpc(executor: &Executor, body: Value) -> Value {
    handle_jsonrpc_with(executor, default_catalog(), body, &McpOptions::default()).await
}

fn default_catalog() -> &'static CatalogService {
    static CATALOG: std::sync::OnceLock<CatalogService> = std::sync::OnceLock::new();
    CATALOG.get_or_init(CatalogService::bundled)
}

/// Handle JSON-RPC with session options.
pub async fn handle_jsonrpc_with(
    executor: &Executor,
    catalog: &CatalogService,
    body: Value,
    opts: &McpOptions,
) -> Value {
    if let Some(arr) = body.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(handle_one(executor, catalog, item.clone(), opts).await);
        }
        return Value::Array(out);
    }
    handle_one(executor, catalog, body, opts).await
}

#[instrument(skip(executor, catalog, body, opts))]
async fn handle_one(
    executor: &Executor,
    catalog: &CatalogService,
    body: Value,
    opts: &McpOptions,
) -> Value {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));
    if body.get("id").is_none() {
        return Value::Null;
    }
    match method {
        "initialize" => ok(
            &id,
            &json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "executor", "version": "0.1.0"},
            }),
        ),
        "ping" => ok(&id, &json!({})),
        "tools/list" => ok(&id, &json!({ "tools": list_tools(executor, opts) })),
        "tools/call" => call_tool(executor, catalog, id, params, opts).await,
        other => rpc_error(&id, -32601, format!("method not found: {other}")),
    }
}

fn list_tools(executor: &Executor, opts: &McpOptions) -> Vec<Value> {
    let mut tools = Vec::new();
    match opts.mode {
        McpMode::Passthrough => {
            tools.push(tool_def(
                "integrations",
                "List connected integrations and accounts. Paginated.",
                &json!({"type":"object","properties":{
                    "limit":{"type":"integer"},
                    "offset":{"type":"integer"}
                }}),
            ));
            tools.push(tool_def(
                "search",
                "Search connected tools. Returns input schemas for matches.",
                &json!({"type":"object","properties":{
                    "query":{"type":"string"},
                    "integration":{"type":"string"},
                    "namespace":{"type":"string"},
                    "limit":{"type":"integer"},
                    "offset":{"type":"integer"}
                }}),
            ));
            tools.push(tool_def(
                "invoke",
                "Invoke a catalog tool by id with JSON arguments.",
                &json!({"type":"object","required":["tool"],"properties":{
                    "tool":{"type":"string"},
                    "arguments":{"type":"object"}
                }}),
            ));
            tools.push(tool_def(
                "skills",
                "Documentation for this server only. Call with no name to list guides, or skills({ name: \"search-invoke\" }).",
                &json!({"type":"object","properties":{"name":{"type":"string"}}}),
            ));
        }
        McpMode::Code => {
            let inventory = integration_inventory(executor, opts);
            tools.push(tool_def(
                "execute",
                &execute_description(&inventory),
                &json!({"type":"object","required":["code"],"properties":{"code":{"type":"string"}}}),
            ));
            tools.push(tool_def(
                "skills",
                "Documentation for THIS server's own tools. Call skills({ name: \"execute\" }) for the full guide. Call with no name to list docs.",
                &json!({"type":"object","properties":{"name":{"type":"string"}}}),
            ));
            match opts.elicitation {
                ElicitationMode::Model => tools.push(tool_def(
                    "resume",
                    "Resume a paused execution using the executionId returned by execute. This connection allows model-side resume via elicitation_mode=model.",
                    &json!({"type":"object","required":["executionId"],"properties":{
                        "executionId":{"type":"string"},
                        "action":{"type":"string","enum":["accept","decline","cancel"]},
                        "content":{"type":"string"},
                        "persist":{"type":"string"}
                    }}),
                )),
                ElicitationMode::Browser => tools.push(tool_def(
                    "resume",
                    "Request user approval to resume a paused execution. Call with the executionId returned by execute.",
                    &json!({"type":"object","required":["executionId"],"properties":{
                        "executionId":{"type":"string"}
                    }}),
                )),
            }
            if opts.search_tools {
                for slug in namespace_slugs(executor, opts) {
                    tools.push(tool_def(
                        &format!("search_{slug}"),
                        "Search this integration's tools; empty query lists all. Run results with execute.",
                        &json!({"type":"object","properties":{"query":{"type":"string"}}}),
                    ));
                }
            }
        }
    }
    push_union_tools(&mut tools);
    tools
}

fn push_union_tools(tools: &mut Vec<Value>) {
    tools.push(tool_def(
        "catalog_search",
        "Search the priced tool catalog by job (what you want to do), not vendor.",
        &json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}}}),
    ));
    tools.push(tool_def(
        "catalog_get",
        "Get one catalog endpoint: params, price in micro-USD, access ladder.",
        &json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
    ));
    tools.push(tool_def(
        "catalog_call",
        "Call a catalog endpoint by id. Own keys are never metered. 402 if the mock balance is empty.",
        &json!({"type":"object","required":["id"],"properties":{
            "id":{"type":"string"},
            "query":{"type":"object"},
            "body":{"type":"object"}
        }}),
    ));
    tools.push(tool_def(
        "connections_list",
        "List saved connections (metadata only; secrets never returned).",
        &json!({"type":"object","properties":{"integration":{"type":"string"}}}),
    ));
}

fn tool_def(name: &str, description: &str, schema: &Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

fn integration_inventory(executor: &Executor, opts: &McpOptions) -> String {
    namespace_slugs(executor, opts)
        .into_iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn namespace_slugs(executor: &Executor, opts: &McpOptions) -> Vec<String> {
    let Ok(rows) = executor.list_integrations() else {
        return Vec::new();
    };
    let mut slugs: Vec<String> = rows
        .into_iter()
        .map(|i| i.slug.as_str().to_owned())
        .filter(|s| slug_in_toolkit(opts, s))
        .filter(|s| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .take(50)
        .collect();
    slugs.sort();
    slugs
}

fn slug_in_toolkit(opts: &McpOptions, slug: &str) -> bool {
    let Some(toolkit) = &opts.toolkit else {
        return true;
    };
    toolkit
        .connections
        .iter()
        .any(|pat| pat == "*" || pat.contains(slug) || glob_starts(pat, slug))
}

fn glob_starts(pat: &str, slug: &str) -> bool {
    pat.trim_start_matches("tools.").starts_with(slug)
}

async fn call_union_tool(
    executor: &Executor,
    catalog: &CatalogService,
    name: &str,
    id: &Value,
    args: &Value,
) -> Option<Value> {
    match name {
        "catalog_search" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = usize::try_from(args.get("limit").and_then(Value::as_u64).unwrap_or(12))
                .unwrap_or(12);
            Some(mcp_ok(
                id,
                &json!({ "items": catalog.catalog().search(query, limit) }),
            ))
        }
        "catalog_get" => {
            let Some(ep_id) = args.get("id").and_then(Value::as_str) else {
                return Some(rpc_error(id, -32602, "catalog_get requires id"));
            };
            Some(catalog.catalog().get(ep_id).map_or_else(
                || rpc_error(id, -32601, format!("endpoint não encontrado: {ep_id}")),
                |ep| mcp_ok(id, &serde_json::to_value(ep).unwrap_or(Value::Null)),
            ))
        }
        "catalog_call" => Some(catalog_call(executor, catalog, id, args).await),
        "connections_list" => Some(connections_list(executor, id, args)),
        _ => None,
    }
}

async fn catalog_call(
    executor: &Executor,
    catalog: &CatalogService,
    id: &Value,
    args: &Value,
) -> Value {
    let Some(ep_id) = args.get("id").and_then(Value::as_str) else {
        return rpc_error(id, -32602, "catalog_call requires id");
    };
    let mut query = std::collections::BTreeMap::new();
    if let Some(obj) = args.get("query").and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                query.insert(k.clone(), s.to_owned());
            } else if !v.is_null() {
                query.insert(k.clone(), v.to_string());
            }
        }
    }
    let connection_secret = catalog
        .catalog()
        .get(ep_id)
        .and_then(|ep| crate::connections::inject_secret(executor, &ep.provider));
    match catalog
        .call(CallInput {
            id: ep_id.to_owned(),
            query,
            body: args.get("body").cloned(),
            subject: LOCAL_SUBJECT.to_owned(),
            connection_secret,
            team_tool_secret: None,
        })
        .await
    {
        Ok(out) => mcp_ok(id, &serde_json::to_value(out).unwrap_or(Value::Null)),
        Err(e) => rpc_error(id, -32003, e.to_string()),
    }
}

fn connections_list(executor: &Executor, id: &Value, args: &Value) -> Value {
    let integration = args
        .get("integration")
        .and_then(Value::as_str)
        .and_then(|s| executor_core::IntegrationSlug::new(s).ok());
    match executor.list_connections(integration.as_ref(), None) {
        Ok(rows) => {
            let items: Vec<Value> = rows
                .into_iter()
                .map(|c| {
                    json!({
                        "owner": c.owner.as_str(),
                        "name": c.name.as_str(),
                        "integration": c.integration.as_str(),
                        "address": c.address.to_string(),
                    })
                })
                .collect();
            mcp_ok(id, &json!({ "connections": items }))
        }
        Err(e) => rpc_error(id, -32003, e.to_string()),
    }
}

async fn call_tool(
    executor: &Executor,
    catalog: &CatalogService,
    id: Value,
    params: Value,
    opts: &McpOptions,
) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return rpc_error(&id, -32602, "missing name");
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(out) = call_union_tool(executor, catalog, name, &id, &args).await {
        return out;
    }
    match name {
        "execute" if opts.mode == McpMode::Code => call_execute(executor, id, args).await,
        "skills" => {
            let skill = args.get("name").and_then(Value::as_str);
            let inventory = integration_inventory(executor, opts);
            mcp_ok(
                &id,
                &skills_result(
                    skill,
                    opts.mode == McpMode::Passthrough,
                    &inventory,
                    &crate::bundles::extra_skills(executor),
                ),
            )
        }
        "resume" if opts.mode == McpMode::Code => call_resume(executor, id, args, opts).await,
        "integrations" if opts.mode == McpMode::Passthrough => {
            mcp_ok(&id, &passthrough_integrations(executor, &args, opts))
        }
        "search" if opts.mode == McpMode::Passthrough => {
            mcp_ok(&id, &passthrough_search(executor, &args, opts))
        }
        "invoke" if opts.mode == McpMode::Passthrough => {
            call_invoke(executor, id, args, opts).await
        }
        other if other.starts_with("search_") && opts.search_tools => {
            let slug = other.trim_start_matches("search_");
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let page = executor.search_ranked(&json!({
                "query": query,
                "namespace": slug,
                "limit": 12,
            }));
            match page {
                Ok(value) => mcp_ok(&id, &value),
                Err(e) => rpc_error(&id, -32000, e.to_string()),
            }
        }
        other => rpc_error(&id, -32601, format!("unknown tool {other}")),
    }
}

async fn call_execute(executor: &Executor, id: Value, args: Value) -> Value {
    let Some(code) = args.get("code").and_then(Value::as_str) else {
        return rpc_error(&id, -32602, "execute requires { code }");
    };
    match executor
        .run_code(
            code,
            ExecuteOptions {
                auto_approve: false,
                timeout: None,
                idempotency_key: None,
            },
        )
        .await
    {
        Ok(outcome) => mcp_ok(&id, &outcome.execution_api()),
        Err(e) => rpc_error(&id, jsonrpc_code(&e), e.to_string()),
    }
}

async fn call_resume(executor: &Executor, id: Value, args: Value, opts: &McpOptions) -> Value {
    let Some(exec_id) = args.get("executionId").and_then(Value::as_str) else {
        return rpc_error(&id, -32602, "resume requires executionId");
    };
    let parsed = match ExecutionId::new(exec_id) {
        Ok(v) => v,
        Err(e) => return rpc_error(&id, -32602, e.to_string()),
    };
    let action = match opts.elicitation {
        ElicitationMode::Browser => ResumeAction::Accept,
        ElicitationMode::Model => match args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("accept")
        {
            "decline" => ResumeAction::Decline,
            "cancel" => ResumeAction::Cancel,
            _ => ResumeAction::Accept,
        },
    };
    let persist = args
        .get("persist")
        .and_then(Value::as_str)
        .and_then(PersistChoice::parse);
    let content = args.get("content").cloned().map(|c| {
        if let Value::String(raw) = &c {
            serde_json::from_str(raw).unwrap_or(c)
        } else {
            c
        }
    });
    match executor
        .resume_request(
            &parsed,
            ResumeRequest {
                action,
                content,
                persist,
            },
        )
        .await
    {
        Ok(outcome) => mcp_ok(&id, &outcome.execution_api()),
        Err(e) => rpc_error(&id, jsonrpc_code(&e), e.to_string()),
    }
}

async fn call_invoke(executor: &Executor, id: Value, args: Value, opts: &McpOptions) -> Value {
    let Some(path) = args.get("tool").and_then(Value::as_str) else {
        return rpc_error(&id, -32602, "invoke requires { tool }");
    };
    if !tool_allowed(executor, path, opts) {
        return rpc_error(&id, -32602, "tool is outside this toolkit");
    }
    let payload = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match executor
        .execute(
            path,
            payload,
            ExecuteOptions {
                auto_approve: false,
                timeout: None,
                idempotency_key: None,
            },
        )
        .await
    {
        Ok(outcome) => mcp_ok(&id, &outcome.execution_api()),
        Err(e) => rpc_error(&id, jsonrpc_code(&e), e.to_string()),
    }
}

fn passthrough_integrations(executor: &Executor, args: &Value, opts: &McpOptions) -> Value {
    let parsed = SearchArgs::from_value(args);
    let Ok(rows) = executor.list_integrations() else {
        return json!({"items":[],"total":0,"hasMore":false,"nextOffset":null});
    };
    let items: Vec<Value> = rows
        .into_iter()
        .filter(|i| slug_in_toolkit(opts, i.slug.as_str()))
        .map(|i| {
            json!({
                "slug": i.slug.as_str(),
                "name": i.name,
                "description": i.description,
                "kind": i.kind.as_str(),
            })
        })
        .collect();
    serde_json::to_value(executor_core::SearchPage::paginate(
        items,
        parsed.offset,
        parsed.limit.max(1),
    ))
    .unwrap_or_else(|_| json!({"items":[]}))
}

fn passthrough_search(executor: &Executor, args: &Value, opts: &McpOptions) -> Value {
    let mut search_args = args.clone();
    if let Some(obj) = search_args.as_object_mut()
        && let Some(integration) = obj.remove("integration")
    {
        obj.entry("namespace").or_insert(integration);
    }
    let Ok(mut page) = executor.search_ranked(&search_args) else {
        return json!({"items":[],"total":0,"hasMore":false,"nextOffset":null});
    };
    if let Some(toolkit) = &opts.toolkit
        && let Some(items) = page.get_mut("items").and_then(Value::as_array_mut)
    {
        items.retain(|item| {
            item.get("path").and_then(Value::as_str).is_some_and(|p| {
                toolkit.allows_address(p) || toolkit.allows_address(&format!("tools.{p}"))
            })
        });
    }
    if let Some(items) = page.get_mut("items").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            if let Some(path) = item
                .get("path")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                && let Ok(tool) = executor.describe(&path)
                && let Some(map) = item.as_object_mut()
            {
                map.insert(
                    "inputSchema".into(),
                    tool.input_schema
                        .clone()
                        .unwrap_or_else(|| json!({"type":"object"})),
                );
            }
        }
    }
    page
}

fn tool_allowed(executor: &Executor, path: &str, opts: &McpOptions) -> bool {
    let Some(toolkit) = &opts.toolkit else {
        return true;
    };
    if toolkit.allows_address(path) || toolkit.allows_address(&format!("tools.{path}")) {
        return true;
    }
    executor.describe(path).is_ok_and(|t| {
        toolkit.allows_address(t.address.to_string().as_str())
            || toolkit.allows_address(&t.cli_path())
    })
}

fn mcp_ok(id: &Value, structured: &Value) -> Value {
    ok(
        id,
        &json!({
            "content": [{"type":"text","text": structured.to_string()}],
            "structuredContent": structured,
        }),
    )
}

const fn jsonrpc_code(err: &executor_core::ExecutorError) -> i64 {
    match err {
        executor_core::ExecutorError::Overloaded { .. } => -32000,
        executor_core::ExecutorError::Timeout { .. } => -32001,
        executor_core::ExecutorError::Cancelled => -32002,
        executor_core::ExecutorError::ToolNotFound { .. } => -32601,
        executor_core::ExecutorError::InvalidArgs(_) => -32602,
        executor_core::ExecutorError::EnterpriseManaged(
            executor_core::EmaError::SubjectTokenRejected { .. },
        ) => -32004,
        _ => -32003,
    }
}

fn ok(id: &Value, result: &Value) -> Value {
    json!({"jsonrpc":"2.0","id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id": id, "error": {"code": code, "message": message.into()}})
}

/// Stdio MCP: newline-delimited JSON (and Content-Length frames).
///
/// # Errors
///
/// IO on stdin/stdout.
pub async fn stdio_loop(executor: Executor) -> Result<(), std::io::Error> {
    stdio_loop_with(executor, McpOptions::default()).await
}

/// Stdio MCP with session options (CLI `--mode` flags).
///
/// # Errors
///
/// IO on stdin/stdout.
pub async fn stdio_loop_with(executor: Executor, opts: McpOptions) -> Result<(), std::io::Error> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Content-Length:") {
            drain_content_length(&mut reader, &mut stdout, &executor, &opts, trimmed).await?;
            continue;
        }
        let Ok(body) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let response = handle_jsonrpc_with(&executor, default_catalog(), body, &opts).await;
        if !response.is_null() {
            let bytes = serde_json::to_vec(&response)?;
            stdout.write_all(&bytes).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

async fn drain_content_length(
    reader: &mut BufReader<tokio::io::Stdin>,
    stdout: &mut tokio::io::Stdout,
    executor: &Executor,
    opts: &McpOptions,
    first: &str,
) -> Result<(), std::io::Error> {
    let mut headers = first.to_owned();
    headers.push('\n');
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
        headers.push_str(&line);
    }
    let len = headers.lines().find_map(|l| {
        l.strip_prefix("Content-Length:")
            .and_then(|s| s.trim().parse::<usize>().ok())
    });
    let Some(len) = len else {
        return Ok(());
    };
    let mut buf = vec![0u8; len];
    tokio::io::AsyncReadExt::read_exact(reader, &mut buf).await?;
    let Ok(body) = serde_json::from_slice::<Value>(&buf) else {
        return Ok(());
    };
    let response = handle_jsonrpc_with(executor, default_catalog(), body, opts).await;
    if response.is_null() {
        return Ok(());
    }
    let payload = serde_json::to_vec(&response)?;
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    stdout.write_all(header.as_bytes()).await?;
    stdout.write_all(&payload).await?;
    stdout.flush().await?;
    Ok(())
}

/// Wrap a JSON-RPC body as a single SSE `message` event.
#[must_use]
pub fn as_sse(body: &Value) -> String {
    format!("event: message\ndata: {body}\n\n")
}

/// SSE ping for Streamable HTTP GET.
#[must_use]
pub fn sse_ping() -> String {
    "event: ping\ndata: {}\n\n".into()
}

/// Shared hub handle for [`crate::AppState`].
pub type SharedMcpHub = Arc<McpHub>;

#[cfg(test)]
mod tests {
    use super::{McpMode, McpOptions, default_catalog, handle_jsonrpc_with};
    use executor_core::Limits;
    use executor_engine::Executor;
    use serde_json::json;

    #[tokio::test]
    async fn default_tools_are_execute_skills_resume() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let listed = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            &McpOptions::default(),
        )
        .await;
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "execute",
                "skills",
                "resume",
                "catalog_search",
                "catalog_get",
                "catalog_call",
                "connections_list"
            ]
        );
    }

    #[tokio::test]
    async fn passthrough_tools() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let listed = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            &McpOptions {
                mode: McpMode::Passthrough,
                ..McpOptions::default()
            },
        )
        .await;
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "integrations",
                "search",
                "invoke",
                "skills",
                "catalog_search",
                "catalog_get",
                "catalog_call",
                "connections_list"
            ]
        );
    }

    #[tokio::test]
    async fn execute_runs_code() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let result = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({
                "jsonrpc":"2.0",
                "id": 2,
                "method":"tools/call",
                "params":{"name":"execute","arguments":{"code":"return 1 + 1;"}}
            }),
            &McpOptions::default(),
        )
        .await;
        assert_eq!(result["result"]["structuredContent"]["status"], "completed");
        assert_eq!(
            result["result"]["structuredContent"]["structured"]["data"],
            2
        );
    }

    #[tokio::test]
    async fn skills_lists_execute() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let result = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({
                "jsonrpc":"2.0",
                "id": 3,
                "method":"tools/call",
                "params":{"name":"skills","arguments":{}}
            }),
            &McpOptions::default(),
        )
        .await;
        let skills = result["result"]["structuredContent"]["skills"]
            .as_array()
            .expect("skills");
        assert_eq!(skills[0]["name"], "execute");
    }

    #[tokio::test]
    async fn catalog_search_by_job() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let result = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({
                "jsonrpc":"2.0",
                "id": 4,
                "method":"tools/call",
                "params":{"name":"catalog_search","arguments":{"query":"encontrar e-mail"}}
            }),
            &McpOptions::default(),
        )
        .await;
        let items = result["result"]["structuredContent"]["items"]
            .as_array()
            .expect("items");
        assert!(
            items.iter().any(|h| h["endpoint"]["id"]
                .as_str()
                .is_some_and(|id| id.contains("email.find"))),
            "{result}"
        );
    }

    #[tokio::test]
    async fn catalog_call_echo() {
        let exec = Executor::builder().limits(Limits::production()).build();
        let result = handle_jsonrpc_with(
            &exec,
            default_catalog(),
            json!({
                "jsonrpc":"2.0",
                "id": 5,
                "method":"tools/call",
                "params":{
                    "name":"catalog_call",
                    "arguments":{"id":"demo.echo","query":{"text":"oi"}}
                }
            }),
            &McpOptions::default(),
        )
        .await;
        assert_eq!(
            result["result"]["structuredContent"]["body"]["text"], "oi",
            "{result}"
        );
        assert_eq!(
            result["result"]["structuredContent"]["served_via"],
            "anonymous"
        );
    }
}
