//! MCP client plugin (HTTP JSON-RPC and stdio).

#![allow(clippy::module_name_repetitions)]

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use executor_core::{
    AuthKind, AuthMethod, Detection, DetectionConfidence, HealthCheckCtx, HealthVerdict,
    IntegrationConfig, IntegrationPlugin, InvokeCtx, PluginError, PluginId, ResolveToolsCtx,
    ResolvedTools, ToolDef, ToolError, ToolName, ToolResult, tool_error_from_http,
};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tracing::instrument;

/// First-party MCP plugin.
pub struct McpPlugin {
    client: Client,
}

impl McpPlugin {
    /// Pooled HTTP client.
    #[must_use]
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }
}

impl Default for McpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IntegrationPlugin for McpPlugin {
    fn id(&self) -> PluginId {
        PluginId::mcp()
    }

    fn detect(&self, candidate: &str) -> Option<Detection> {
        let lower = candidate.to_ascii_lowercase();
        if lower.contains("mcp") || lower.starts_with("stdio:") {
            Some(Detection {
                kind: PluginId::mcp(),
                confidence: DetectionConfidence::Medium,
                endpoint: candidate.to_owned(),
                name: "MCP".into(),
                slug: "mcp".into(),
            })
        } else {
            None
        }
    }

    fn describe_auth(&self, config: &IntegrationConfig) -> Vec<AuthMethod> {
        if config.get("command").is_some() {
            return vec![AuthMethod::none()];
        }
        vec![AuthMethod::none(), AuthMethod::bearer()]
    }

    #[instrument(skip(self, ctx))]
    async fn resolve_tools(&self, ctx: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError> {
        let listed = list_tools(
            &self.client,
            ctx.config,
            ctx.values,
            ctx.timeout,
            ctx.max_spec_bytes,
        )
        .await?;
        if listed.len() > ctx.max_tools {
            return Err(PluginError::new(format!(
                "MCP server listed {} tools (max {})",
                listed.len(),
                ctx.max_tools
            )));
        }
        Ok(ResolvedTools {
            tools: listed,
            definitions: None,
            incomplete: false,
            incomplete_reason: None,
        })
    }

    #[instrument(skip(self, ctx))]
    async fn invoke(&self, ctx: InvokeCtx<'_>) -> Result<ToolResult, PluginError> {
        let name = ctx.tool.name.as_str();
        call_tool(
            &self.client,
            &ctx.integration.config,
            ctx.values,
            ctx.template.as_str(),
            &ctx.integration.integration.auth_methods,
            name,
            ctx.args,
            ctx.timeout,
        )
        .await
    }

    async fn check_health(&self, _ctx: HealthCheckCtx<'_>) -> Result<HealthVerdict, PluginError> {
        Ok(HealthVerdict::Unknown)
    }
}

async fn list_tools(
    client: &Client,
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    timeout: Duration,
    max_spec_bytes: usize,
) -> Result<Vec<ToolDef>, PluginError> {
    let result = rpc(
        client,
        config,
        values,
        "none",
        &[],
        "tools/list",
        &json!({}),
        timeout,
    )
    .await?;
    let encoded = serde_json::to_vec(&result).unwrap_or_default().len();
    if encoded > max_spec_bytes {
        return Err(PluginError::new(format!(
            "tools/list is {encoded} bytes (max {max_spec_bytes})"
        )));
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for t in tools {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError::new("MCP tool missing name"))?;
        out.push(ToolDef {
            name: ToolName::new(name).map_err(PluginError::new)?,
            description: t
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            input_schema: t.get("inputSchema").cloned(),
            output_schema: None,
            annotations: None,
            plugin_meta: Some(json!({"mcpName": name})),
        });
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)] // RPC envelope: transport + auth + method.
async fn call_tool(
    client: &Client,
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    template: &str,
    methods: &[AuthMethod],
    name: &str,
    args: &Value,
    timeout: Duration,
) -> Result<ToolResult, PluginError> {
    let transport = config
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("http");
    let stdio = transport == "stdio" || (config.get("command").is_some() && transport != "http");
    let result = if stdio {
        stdio_rpc(
            config,
            values,
            "tools/call",
            &json!({"name": name, "arguments": args}),
            timeout,
        )
        .await
        .map_err(|e| ToolError {
            code: "mcp_error".into(),
            message: e.0,
            status: None,
            details: None,
            retryable: Some(true),
        })
    } else {
        http_rpc(
            client,
            config,
            values,
            template,
            methods,
            "tools/call",
            &json!({"name": name, "arguments": args}),
            timeout,
        )
        .await
    };
    match result {
        Ok(result) => {
            if result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                Ok(ToolResult::fail(ToolError {
                    code: "mcp_error".into(),
                    message: result.to_string(),
                    status: None,
                    details: Some(result),
                    retryable: Some(false),
                }))
            } else {
                Ok(ToolResult::ok(result))
            }
        }
        Err(error) => Ok(ToolResult::fail(error)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn rpc(
    client: &Client,
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    template: &str,
    methods: &[AuthMethod],
    method: &str,
    params: &Value,
    timeout: Duration,
) -> Result<Value, PluginError> {
    let transport = config
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("http");
    let stdio = transport == "stdio" || (config.get("command").is_some() && transport != "http");
    if stdio {
        return stdio_rpc(config, values, method, params, timeout).await;
    }
    http_rpc(
        client, config, values, template, methods, method, params, timeout,
    )
    .await
    .map_err(|e| PluginError::new(e.message))
}

#[allow(clippy::too_many_arguments)]
async fn http_rpc(
    client: &Client,
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    template: &str,
    methods: &[AuthMethod],
    method: &str,
    params: &Value,
    timeout: Duration,
) -> Result<Value, ToolError> {
    let url = config
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError {
            code: "mcp_error".into(),
            message: "MCP http transport requires url".into(),
            status: None,
            details: None,
            retryable: Some(false),
        })?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let mut req = client
        .post(url)
        .timeout(timeout)
        .header("content-type", "application/json")
        .json(&body);
    let method_auth = methods
        .iter()
        .find(|m| m.template.as_str() == template || m.id == template)
        .cloned()
        .unwrap_or_else(AuthMethod::none);
    if (method_auth.kind == AuthKind::Header || method_auth.kind == AuthKind::Oauth)
        && let Some(token) = values.get("token")
    {
        req = req.bearer_auth(token);
    }
    for p in &method_auth.placements {
        if p.carrier == executor_core::Carrier::Header
            && let Some(v) = p
                .literal
                .clone()
                .or_else(|| values.get(&p.variable).cloned())
        {
            req = req.header(&p.name, format!("{}{v}", p.prefix));
        }
    }
    let response = req.send().await.map_err(|e| ToolError {
        code: "mcp_error".into(),
        message: format!("mcp http: {e}"),
        status: None,
        details: None,
        retryable: Some(true),
    })?;
    let status = response.status();
    let header_pairs: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let bytes = response.bytes().await.map_err(|e| ToolError {
        code: "mcp_error".into(),
        message: format!("mcp http body: {e}"),
        status: None,
        details: None,
        retryable: Some(true),
    })?;
    let rpc: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
    if !status.is_success() {
        return Err(tool_error_from_http(status.as_u16(), &header_pairs, &rpc));
    }
    if let Some(err) = rpc.get("error") {
        return Err(ToolError {
            code: "mcp_error".into(),
            message: err.to_string(),
            status: None,
            details: Some(err.clone()),
            retryable: Some(false),
        });
    }
    Ok(rpc.get("result").cloned().unwrap_or(Value::Null))
}

async fn stdio_rpc(
    config: &IntegrationConfig,
    values: &executor_core::CredentialMapValues,
    method: &str,
    params: &Value,
    timeout: Duration,
) -> Result<Value, PluginError> {
    let command = config
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| PluginError::new("MCP stdio transport requires command"))?;
    let args: Vec<String> = config
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut child = Command::new(command);
    child
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in values {
        child.env(k, v);
    }
    let mut child = child
        .spawn()
        .map_err(|e| PluginError::new(format!("spawn mcp: {e}")))?;
    let init_params = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "executor", "version": "0.1.0"},
    });
    let init = jsonrpc("initialize", &init_params);
    write_frame(&mut child, &init).await?;
    let _ = read_frame(&mut child, timeout).await?;
    write_frame(
        &mut child,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    write_frame(&mut child, &jsonrpc(method, params)).await?;
    let rpc = read_frame(&mut child, timeout).await?;
    let _ = child.kill().await;
    if let Some(err) = rpc.get("error") {
        return Err(PluginError::new(err.to_string()));
    }
    Ok(rpc.get("result").cloned().unwrap_or(Value::Null))
}

fn jsonrpc(method: &str, params: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"method": method, "params": params})
}

async fn write_frame(child: &mut Child, body: &Value) -> Result<(), PluginError> {
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| PluginError::new("mcp stdin closed"))?;
    let payload = serde_json::to_vec(body).map_err(PluginError::new)?;
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    stdin
        .write_all(header.as_bytes())
        .await
        .map_err(PluginError::new)?;
    stdin.write_all(&payload).await.map_err(PluginError::new)?;
    stdin.flush().await.map_err(PluginError::new)?;
    Ok(())
}

async fn read_frame(child: &mut Child, timeout: Duration) -> Result<Value, PluginError> {
    let stdout = child
        .stdout
        .as_mut()
        .ok_or_else(|| PluginError::new("mcp stdout closed"))?;
    let read = async {
        let mut buf = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            stdout
                .read_exact(&mut byte)
                .await
                .map_err(PluginError::new)?;
            buf.push(byte[0]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if buf.len() > 8192 {
                return Err(PluginError::new("mcp header too large"));
            }
        }
        let header = String::from_utf8_lossy(&buf);
        let len = header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .and_then(|s| s.trim().parse::<usize>().ok())
            .ok_or_else(|| PluginError::new("mcp missing Content-Length"))?;
        if len > 8 * 1024 * 1024 {
            return Err(PluginError::new("mcp frame exceeds 8MiB"));
        }
        let mut body = vec![0u8; len];
        stdout
            .read_exact(&mut body)
            .await
            .map_err(PluginError::new)?;
        serde_json::from_slice(&body).map_err(PluginError::new)
    };
    tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| PluginError::new("mcp stdio timeout"))?
}

#[cfg(test)]
mod tests {
    use super::McpPlugin;
    use executor_core::IntegrationPlugin;

    #[test]
    fn detects_mcp() {
        let p = McpPlugin::new();
        assert!(p.detect("https://example.test/mcp").is_some());
    }
}
