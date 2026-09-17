//! `executor` CLI — catalog, call, connections, resume, MCP, daemon, login, console.

#![allow(clippy::module_name_repetitions)]

mod call_help;
mod daemon;
mod login;
mod mcp_bridge;
mod profiles;
mod service;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use executor_core::{compile_call, resolve_invocation, unix_now_ms};
use executor_host::{
    AppState, DEFAULT_PORT, DEFAULT_SERVICE_PORT, HostConfig, default_allowed_hosts,
    default_console_origin, load_or_mint_auth_with, serve,
};
use executor_sdk::{
    CreateOptions, apply_config, create_executor_with_metrics, data_dir, load_jsonc,
};
use executor_storage::DataDirLock;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// Executor ∪ Treg — integration catalog and priced tool gateway.
#[derive(Parser, Debug)]
#[command(
    name = "executor",
    version,
    about = "Catálogo de integrações e ferramentas com preço (Executor ∪ Treg)"
)]
struct Cli {
    /// Catalog directory (overrides `EXECUTOR_DATA_DIR`).
    #[arg(long, env = "EXECUTOR_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Invoke a tool, or run a code-mode script with `--code`.
    #[command(disable_help_flag = true)]
    Call {
        /// Path segments and optional trailing JSON / `@file.json`.
        path: Vec<String>,
        /// Namespace browse (`executor call --help github`).
        #[arg(short = 'h', long = "help")]
        show_help: bool,
        /// Substring filter for `--help` children.
        #[arg(long = "match")]
        match_query: Option<String>,
        /// Max children to print with `--help`.
        #[arg(long)]
        limit: Option<usize>,
        /// Bounded code-mode source (`return await tools["path"]({})`).
        #[arg(long)]
        code: Option<String>,
        /// Skip approval pauses.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Idempotency key for retries.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Resume a paused execution.
    Resume {
        /// Execution id.
        #[arg(long)]
        execution_id: String,
        /// accept | decline | cancel
        #[arg(long, default_value = "accept")]
        action: String,
        /// JSON content for form elicitations.
        #[arg(long)]
        content: Option<String>,
        /// Persist-choice: `session` or `always`.
        #[arg(long)]
        persist: Option<String>,
    },
    /// Tool catalog.
    Tools {
        #[command(subcommand)]
        cmd: ToolsCmd,
    },
    /// MCP stdio host (always bridges to the daemon `/mcp`).
    Mcp {
        /// `code` (default) or `passthrough`.
        #[arg(long, default_value = "code")]
        mode: String,
        /// `browser` (default) or `model`.
        #[arg(long, default_value = "browser")]
        elicitation_mode: String,
        /// Register `search_<integration>` tools.
        #[arg(long)]
        search_tools: bool,
    },
    /// Foreground HTTP daemon (loopback).
    Serve {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Daemon process control.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Write an OS service unit (systemd --user / launchd / schtasks).
    Install {
        /// Best-effort boot-before-login (linger / ONSTART).
        #[arg(long)]
        boot: bool,
    },
    /// Remove the systemd user unit.
    Uninstall,
    /// RFC 8628 device login (prints a verification URL).
    Login {
        /// Profile name (default `default`).
        #[arg(long)]
        server: Option<String>,
        /// Server origin.
        #[arg(long)]
        base_url: Option<String>,
        /// Print the URL and exit without polling.
        #[arg(long)]
        no_poll: bool,
        /// Do not spawn xdg-open.
        #[arg(long)]
        no_open: bool,
    },
    /// Drop stored tokens for a profile.
    Logout {
        #[arg(long)]
        server: Option<String>,
    },
    /// Show the active profile identity.
    Whoami {
        #[arg(long)]
        server: Option<String>,
    },
    /// Named remote server profiles.
    Server {
        #[command(subcommand)]
        cmd: ServerCmd,
    },
    /// OS service aliases for daemon install/status.
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    /// Print the loopback daemon health URL.
    Open {
        #[arg(long)]
        no_open: bool,
    },
    /// Print the project documentation URL.
    Docs {
        #[arg(long)]
        no_open: bool,
    },
    /// Open the pt-BR console (ensure daemon, print origin).
    Web {
        #[arg(long)]
        no_open: bool,
        /// Console origin (default `EXECUTOR_CONSOLE_ORIGIN` or :43123).
        #[arg(long)]
        origin: Option<String>,
    },
    /// Search the priced catalog by job.
    Catalog {
        #[command(subcommand)]
        cmd: Option<CatalogCmd>,
    },
    /// Saved connections (secrets never printed).
    Connections {
        #[command(subcommand)]
        cmd: ConnectionsCmd,
    },
    /// Prepaid mock balance (micro-USD).
    Balance,
    /// Team-owned relay tools.
    #[command(name = "team-tool")]
    TeamTool {
        #[command(subcommand)]
        cmd: TeamToolCmd,
    },
}

#[derive(Subcommand, Debug)]
enum ToolsCmd {
    /// List tools (blocked omitted).
    List,
    /// Search name/description.
    Search { query: String },
    /// List integrations.
    Integrations,
    /// Print one tool's schema.
    Describe { path: Vec<String> },
}

#[derive(Subcommand, Debug)]
enum DaemonCmd {
    /// Run the daemon (detached unless `--foreground`).
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        foreground: bool,
        /// Bind hostname (`127.0.0.1`, `0.0.0.0`, `localhost`).
        #[arg(long, default_value = "127.0.0.1")]
        hostname: String,
        /// Extra CORS origins (repeatable).
        #[arg(long = "allowed-host")]
        allowed_host: Vec<String>,
        /// Override the minted `auth.json` bearer.
        #[arg(long)]
        auth_token: Option<String>,
    },
    /// Print pid/status.
    Status,
    /// Stop via pid file.
    Stop,
    /// Stop then run.
    Restart {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ServerCmd {
    /// Add or replace a profile.
    Add {
        name: String,
        #[arg(long)]
        origin: String,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        default: bool,
    },
    /// List profiles.
    List,
    /// Select the default profile.
    Use { name: String },
    /// Delete a profile.
    Remove { name: String },
    /// Rotate a local bearer token on the profile.
    RotateToken { name: String },
}

#[derive(Subcommand, Debug)]
enum CatalogCmd {
    /// Search by job (`encontrar e-mail`).
    Search {
        /// Job phrase.
        #[arg(required = true, trailing_var_arg = true)]
        job: Vec<String>,
    },
    /// One endpoint: schema + price.
    Get {
        /// Catalog id (`hunter.people.email.find`).
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum ConnectionsCmd {
    /// List metadata (no secret values).
    List,
    /// Create a connection (`--value token=…`).
    Add {
        integration: String,
        name: String,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long, default_value = "bearer")]
        template: String,
        #[arg(long)]
        identity_label: Option<String>,
        /// Repeatable `KEY=VALUE` (write-only).
        #[arg(long = "value", value_name = "KEY=VALUE")]
        values: Vec<String>,
    },
    /// Delete `owner integration name`.
    Remove {
        owner: String,
        integration: String,
        name: String,
    },
    /// Re-resolve tools.
    Refresh {
        owner: String,
        integration: String,
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum TeamToolCmd {
    /// List team tools (no secrets).
    List,
    /// Register a relay (`base_url` + secret).
    Add {
        name: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        secret: String,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceCmd {
    /// Alias of `install`.
    Install {
        #[arg(long)]
        boot: bool,
    },
    /// Alias of `uninstall`.
    Uninstall,
    /// Alias of `daemon status`.
    Status,
    /// Alias of `daemon restart`.
    Restart {
        #[arg(long, default_value_t = DEFAULT_SERVICE_PORT)]
        port: u16,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_lines)] // clap dispatch
async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = cli.data_dir.clone();
    match cli.command {
        Commands::Call {
            path,
            show_help,
            match_query,
            limit,
            code,
            yes,
            idempotency_key,
        } => {
            if show_help {
                let origin = daemon::ensure_daemon(dir.as_deref()).await?;
                let token = daemon::bearer_token(dir.as_deref());
                call_help::run(
                    &origin,
                    token.as_deref(),
                    &path,
                    match_query.as_deref(),
                    limit,
                )
                .await
            } else {
                cmd_call(dir.as_deref(), path, code, yes, idempotency_key).await
            }
        }
        Commands::Resume {
            execution_id,
            action,
            content,
            persist,
        } => cmd_resume(dir.as_deref(), execution_id, action, content, persist).await,
        Commands::Tools { cmd } => cmd_tools(dir.as_deref(), cmd).await,
        Commands::Mcp {
            mode,
            elicitation_mode,
            search_tools,
        } => {
            let origin = daemon::ensure_daemon(dir.as_deref()).await?;
            let token = daemon::bearer_token(dir.as_deref());
            let mut q = format!("?mode={mode}&elicitation_mode={elicitation_mode}");
            if search_tools {
                q.push_str("&search_tools=true");
            }
            mcp_bridge::run(&origin, &q, token.as_deref()).await?;
            Ok(())
        }
        Commands::Serve { port } => {
            daemon_run(DaemonRun {
                dir: dir.as_deref(),
                port,
                foreground: true,
                hostname: "127.0.0.1".into(),
                allowed_hosts: Vec::new(),
                auth_token: None,
            })
            .await
        }
        Commands::Daemon { cmd } => daemon_cmd(dir.as_deref(), cmd).await,
        Commands::Install { boot }
        | Commands::Service {
            cmd: ServiceCmd::Install { boot },
        } => service::install(dir.as_deref(), boot),
        Commands::Uninstall
        | Commands::Service {
            cmd: ServiceCmd::Uninstall,
        } => service::uninstall(),
        Commands::Service {
            cmd: ServiceCmd::Status,
        } => {
            daemon_status(dir.as_deref());
            Ok(())
        }
        Commands::Service {
            cmd: ServiceCmd::Restart { port },
        } => {
            daemon::stop(dir.as_deref());
            daemon_run(DaemonRun {
                dir: dir.as_deref(),
                port,
                foreground: true,
                hostname: "127.0.0.1".into(),
                allowed_hosts: Vec::new(),
                auth_token: None,
            })
            .await
        }
        Commands::Login {
            server,
            base_url,
            no_poll,
            no_open,
        } => cmd_login(dir.as_deref(), server, base_url, no_poll, no_open).await,
        Commands::Logout { server } => cmd_logout(dir.as_deref(), server),
        Commands::Whoami { server } => cmd_whoami(dir.as_deref(), server),
        Commands::Server { cmd } => cmd_server(dir.as_deref(), cmd),
        Commands::Open { no_open } => {
            cmd_open(no_open);
            Ok(())
        }
        Commands::Docs { no_open } => {
            cmd_docs(no_open);
            Ok(())
        }
        Commands::Web { no_open, origin } => {
            cmd_web(dir.as_deref(), no_open, origin).await?;
            Ok(())
        }
        Commands::Catalog { cmd } => cmd_catalog(dir.as_deref(), cmd).await,
        Commands::Connections { cmd } => cmd_connections(dir.as_deref(), cmd).await,
        Commands::Balance => cmd_balance(dir.as_deref()).await,
        Commands::TeamTool { cmd } => cmd_team_tool(dir.as_deref(), cmd).await,
    }
}

async fn daemon_cmd(
    dir: Option<&Path>,
    cmd: DaemonCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        DaemonCmd::Run {
            port,
            foreground,
            hostname,
            allowed_host,
            auth_token,
        } => {
            daemon_run(DaemonRun {
                dir,
                port,
                foreground,
                hostname,
                allowed_hosts: allowed_host,
                auth_token,
            })
            .await
        }
        DaemonCmd::Status => {
            daemon_status(dir);
            Ok(())
        }
        DaemonCmd::Stop => {
            daemon::stop(dir);
            Ok(())
        }
        DaemonCmd::Restart { port } => {
            daemon::stop(dir);
            daemon_run(DaemonRun {
                dir,
                port,
                foreground: true,
                hostname: "127.0.0.1".into(),
                allowed_hosts: Vec::new(),
                auth_token: None,
            })
            .await
        }
    }
}

async fn cmd_call(
    dir: Option<&Path>,
    path: Vec<String>,
    code: Option<String>,
    yes: bool,
    idempotency_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    if code.is_none()
        && looks_like_catalog_id(&path)
        && let Some(id) = path.first()
    {
        let encoded = urlencoding_query(id);
        if daemon::get_json(
            &origin,
            &format!("/api/catalog/{encoded}"),
            token.as_deref(),
        )
        .await
        .is_ok()
        {
            let query = catalog_query_from_path(&path)?;
            let body = json!({ "id": id, "query": query });
            let outcome = daemon::post_json(&origin, "/api/call", &body, token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
            return Ok(());
        }
    }
    let source = if let Some(code) = code {
        code
    } else {
        let invocation = resolve_invocation(&path)?;
        compile_call(&invocation.path, &Value::Object(invocation.args))
    };
    let mut body = json!({
        "code": source,
        "autoApprove": yes,
    });
    if let Some(key) = idempotency_key {
        body["idempotencyKey"] = json!(key);
    }
    let outcome = daemon::post_json(&origin, "/executions", &body, token.as_deref()).await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

async fn cmd_resume(
    dir: Option<&Path>,
    execution_id: String,
    action: String,
    content: Option<String>,
    persist: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    let mut body = json!({ "action": action });
    if let Some(raw) = content {
        body["content"] = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    }
    if let Some(persist) = persist {
        body["persist"] = json!(persist);
    }
    let path = format!("/executions/{execution_id}/resume");
    let outcome = daemon::post_json(&origin, &path, &body, token.as_deref()).await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

async fn cmd_tools(
    dir: Option<&Path>,
    cmd: ToolsCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    match cmd {
        ToolsCmd::List => {
            let body = daemon::get_json(&origin, "/api/tools", token.as_deref()).await?;
            print_tool_rows(&body);
        }
        ToolsCmd::Search { query } => {
            let q = urlencoding_query(&query);
            let body =
                daemon::get_json(&origin, &format!("/api/tools?q={q}"), token.as_deref()).await?;
            if body
                .get("tools")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
            {
                println!("(no matching tools)");
            } else {
                print_tool_rows(&body);
            }
        }
        ToolsCmd::Integrations => {
            let body = daemon::get_json(&origin, "/api/integrations", token.as_deref()).await?;
            let rows = body
                .get("integrations")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if rows.is_empty() {
                println!("(no integrations)");
            } else {
                for i in rows {
                    println!(
                        "{}\t{}\t{}",
                        i.get("slug").and_then(Value::as_str).unwrap_or(""),
                        i.get("kind").and_then(Value::as_str).unwrap_or(""),
                        i.get("name").and_then(Value::as_str).unwrap_or("")
                    );
                }
            }
        }
        ToolsCmd::Describe { path } => {
            let joined = path.join(".");
            let q = urlencoding_query(&joined);
            let body =
                daemon::get_json(&origin, &format!("/api/tools?q={q}"), token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
    }
    Ok(())
}

fn print_tool_rows(body: &Value) {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return;
    };
    for t in tools {
        println!(
            "{}\t{}",
            t.get("path").and_then(Value::as_str).unwrap_or(""),
            t.get("description").and_then(Value::as_str).unwrap_or("")
        );
    }
}

pub(crate) fn urlencoding_query(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0f));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    char::from(if n < 10 { b'0' + n } else { b'A' + (n - 10) })
}

fn bind_addr(hostname: &str, port: u16) -> SocketAddr {
    let host = std::env::var("EXECUTOR_BIND").unwrap_or_else(|_| hostname.to_owned());
    if host.eq_ignore_ascii_case("localhost") {
        return SocketAddr::from((IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    let ip: IpAddr = host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    SocketAddr::from((ip, port))
}

struct DaemonRun<'a> {
    dir: Option<&'a Path>,
    port: u16,
    foreground: bool,
    hostname: String,
    allowed_hosts: Vec<String>,
    auth_token: Option<String>,
}

async fn daemon_run(opts: DaemonRun<'_>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = data_dir(opts.dir);
    let origin_host = if opts.hostname == "0.0.0.0" || opts.hostname == "::" {
        "127.0.0.1"
    } else {
        opts.hostname.as_str()
    };
    let origin = format!("http://{origin_host}:{}", opts.port);
    if !opts.foreground {
        if daemon::is_healthy(&origin).await {
            println!("{origin}");
            return Ok(());
        }
        daemon::spawn_daemon(&data, opts.port, &opts.hostname, &opts.allowed_hosts)?;
        for _ in 0..80 {
            if daemon::is_healthy(&origin).await {
                println!("{origin}");
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        return Err(format!("daemon did not become healthy at {origin}").into());
    }
    let ownership = DataDirLock::acquire(&data)?;
    let token = load_or_mint_auth_with(&data, opts.auth_token.as_deref())?;
    let mut allowed = default_allowed_hosts();
    allowed.push(opts.hostname.clone());
    allowed.extend(opts.allowed_hosts.iter().cloned());
    if opts.hostname == "0.0.0.0" || opts.hostname == "::" {
        allowed.push("*".into());
    }
    allowed.sort();
    allowed.dedup();
    let metrics = Arc::new(executor_core::AtomicMetrics::new());
    let (sink, sentry) =
        executor_host::attach_sentry(Arc::clone(&metrics) as Arc<dyn executor_core::Metrics>);
    let exec = create_executor_with_metrics(
        CreateOptions {
            data_dir: Some(data.clone()),
            ..CreateOptions::default()
        },
        sink,
    )?;
    if let Some((path, cfg)) = load_jsonc(Some(&data)) {
        tracing::info!(path = %path.display(), "applying executor.jsonc");
        apply_config(&exec, &cfg).await?;
    }
    let cancel = exec.cancellation_token();
    let bind = bind_addr(&opts.hostname, opts.port);
    let pid_path = data.join("daemon.pid");
    write_pid(&pid_path)?;
    let ctrlc_task = ctrlc(cancel.clone());
    tracing::info!(%bind, "executor daemon listening");
    let pointer = daemon::DaemonPointer {
        origin: origin.clone(),
        pid: std::process::id(),
    };
    let _ = std::fs::create_dir_all(&data);
    let _ = std::fs::write(
        daemon::pointer_path(Some(&data)),
        serde_json::to_vec_pretty(&pointer).unwrap_or_default(),
    );
    let state = AppState::new(exec, Some(metrics)).with_control(Some(token), allowed, origin);
    let result = serve(
        HostConfig {
            bind,
            limits: state.executor.limits().clone(),
            auth_token: state.auth_token.clone(),
            allowed_hosts: state.allowed_hosts.clone(),
        },
        state,
        cancel,
    )
    .await;
    let _ = std::fs::remove_file(pid_path);
    drop(ctrlc_task);
    result?;
    drop(sentry);
    drop(ownership);
    Ok(())
}

fn daemon_status(dir: Option<&Path>) {
    let data = data_dir(dir);
    if let Ok(text) = std::fs::read_to_string(daemon::pointer_path(Some(&data))) {
        println!("{text}");
        return;
    }
    let pid_path = data.join("daemon.pid");
    match std::fs::read_to_string(&pid_path) {
        Ok(s) => println!("pid {} ({})", s.trim(), pid_path.display()),
        Err(_) => println!("daemon is not running"),
    }
}

fn write_pid(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", std::process::id()))?;
    Ok(())
}

fn ctrlc(cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel.cancel();
    })
}

async fn cmd_login(
    dir: Option<&Path>,
    server: Option<String>,
    base_url: Option<String>,
    no_poll: bool,
    no_open: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = data_dir(dir);
    let profile = server.unwrap_or_else(|| "default".into());
    let origin = base_url.unwrap_or_else(|| format!("http://127.0.0.1:{DEFAULT_PORT}"));
    let discovery = login::discover(&origin).await?;
    let grant = login::request_device_code(&discovery).await?;
    let user_code = grant.get("user_code").and_then(Value::as_str).unwrap_or("");
    let verify = grant
        .get("verification_uri")
        .or_else(|| grant.get("verification_uri_complete"))
        .and_then(Value::as_str)
        .unwrap_or("");
    println!("user_code: {user_code}");
    println!("open: {verify}");
    if !no_open {
        login::try_open(verify);
    }
    if no_poll {
        return Ok(());
    }
    let expires = grant
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(300);
    let interval = grant.get("interval").and_then(Value::as_u64).unwrap_or(1);
    let device_code = grant
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or("device authorization response missing device_code")?;
    let tokens = login::poll_token(&discovery, device_code, expires, interval).await?;
    login::store_tokens(&data, &profile, &origin, &tokens)?;
    println!("logged in as profile {profile}");
    Ok(())
}

fn cmd_logout(
    dir: Option<&Path>,
    server: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = data_dir(dir);
    let name = server.unwrap_or_else(|| "default".into());
    profiles::remove(&data, &name)?;
    println!("logged out {name}");
    Ok(())
}

fn cmd_whoami(
    dir: Option<&Path>,
    server: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let store = profiles::load(&data_dir(dir))?;
    let profile = if let Some(name) = server {
        store
            .profiles
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("No server profile named \"{name}\"."))?
    } else {
        profiles::active(&store).ok_or("no server profile; run executor login")?
    };
    let identity = match &profile.connection.auth {
        Some(profiles::Auth::Oauth { email, .. }) => {
            email.clone().unwrap_or_else(|| "(oauth)".into())
        }
        Some(profiles::Auth::Bearer { .. }) => "(bearer)".into(),
        None => "(anonymous)".into(),
    };
    println!(
        "{}\t{}\t{identity}",
        profile.name, profile.connection.origin
    );
    Ok(())
}

fn cmd_server(
    dir: Option<&Path>,
    cmd: ServerCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let data = data_dir(dir);
    match cmd {
        ServerCmd::Add {
            name,
            origin,
            token,
            default,
        } => {
            let auth = token.map(|token| profiles::Auth::Bearer { token });
            profiles::upsert(&data, &name, &origin, auth, default)?;
            println!("saved profile {name}");
        }
        ServerCmd::List => {
            let store = profiles::load(&data)?;
            if store.profiles.is_empty() {
                println!("(no server profiles)");
            } else {
                for p in store.profiles {
                    let mark = if store.default_profile.as_deref() == Some(p.name.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    println!("{mark} {}\t{}", p.name, p.connection.origin);
                }
            }
        }
        ServerCmd::Use { name } => {
            profiles::set_default(&data, &name)?;
            println!("using {name}");
        }
        ServerCmd::Remove { name } => {
            profiles::remove(&data, &name)?;
            println!("removed {name}");
        }
        ServerCmd::RotateToken { name } => {
            let token = format!("{:x}-{:x}", unix_now_ms(), std::process::id());
            let store = profiles::load(&data)?;
            let origin = store
                .profiles
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.connection.origin.clone())
                .ok_or_else(|| format!("No server profile named \"{name}\"."))?;
            profiles::upsert(
                &data,
                &name,
                &origin,
                Some(profiles::Auth::Bearer {
                    token: token.clone(),
                }),
                false,
            )?;
            println!("{token}");
        }
    }
    Ok(())
}

fn cmd_open(no_open: bool) {
    let url = format!("http://127.0.0.1:{DEFAULT_PORT}/api/health");
    println!("{url}");
    if !no_open {
        login::try_open(&url);
    }
}

fn cmd_docs(no_open: bool) {
    let url = "https://github.com/UsefulSoftwareCo/executor";
    println!("{url}");
    if !no_open {
        login::try_open(url);
    }
}

fn looks_like_catalog_id(path: &[String]) -> bool {
    let Some(first) = path.first() else {
        return false;
    };
    !first.starts_with("tools.") && !first.starts_with("executor.") && first.contains('.')
}

fn catalog_query_from_path(
    path: &[String],
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let last = path.last().cloned().unwrap_or_default();
    let json_text = if let Some(file) = last.strip_prefix('@') {
        std::fs::read_to_string(file)?
    } else {
        last
    };
    let trimmed = json_text.trim();
    if !trimmed.starts_with('{') {
        return Ok(BTreeMap::new());
    }
    let value: Value = serde_json::from_str(trimmed)?;
    let mut query = BTreeMap::new();
    if let Some(obj) = value.as_object() {
        for (key, val) in obj {
            match val {
                Value::String(s) => {
                    query.insert(key.clone(), s.clone());
                }
                Value::Null => {}
                other => {
                    query.insert(key.clone(), other.to_string());
                }
            }
        }
    }
    Ok(query)
}

fn parse_values(
    pairs: &[String],
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut values = BTreeMap::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!("--value deve ser KEY=VALUE, recebido {pair}").into());
        };
        values.insert(key.to_owned(), value.to_owned());
    }
    Ok(values)
}

async fn cmd_web(
    dir: Option<&Path>,
    no_open: bool,
    origin: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _daemon = daemon::ensure_daemon(dir).await?;
    let url = origin.unwrap_or_else(default_console_origin);
    println!("{url}");
    if !no_open {
        login::try_open(&url);
    }
    Ok(())
}

async fn cmd_catalog(
    dir: Option<&Path>,
    cmd: Option<CatalogCmd>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    match cmd {
        None => {
            let body = daemon::get_json(&origin, "/api/catalog", token.as_deref()).await?;
            let mut providers: Vec<String> = body
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|h| h.pointer("/endpoint/provider")?.as_str().map(str::to_owned))
                .collect();
            providers.sort();
            providers.dedup();
            if providers.is_empty() {
                println!("(nenhum provedor no catálogo)");
            } else {
                for p in providers {
                    println!("{p}");
                }
            }
        }
        Some(CatalogCmd::Search { job }) => {
            let q = urlencoding_query(&job.join(" "));
            let body =
                daemon::get_json(&origin, &format!("/api/catalog?q={q}"), token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        Some(CatalogCmd::Get { id }) => {
            let encoded = urlencoding_query(&id);
            let body = daemon::get_json(
                &origin,
                &format!("/api/catalog/{encoded}"),
                token.as_deref(),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
    }
    Ok(())
}

async fn cmd_connections(
    dir: Option<&Path>,
    cmd: ConnectionsCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    match cmd {
        ConnectionsCmd::List => {
            let body = daemon::get_json(&origin, "/api/connections", token.as_deref()).await?;
            let rows = body
                .get("connections")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if rows.is_empty() {
                println!("(nenhuma conexão)");
            } else {
                for c in rows {
                    println!(
                        "{}/{}/{}\t{}",
                        c.get("owner").and_then(Value::as_str).unwrap_or(""),
                        c.get("integration").and_then(Value::as_str).unwrap_or(""),
                        c.get("name").and_then(Value::as_str).unwrap_or(""),
                        c.get("template").and_then(Value::as_str).unwrap_or("")
                    );
                }
            }
        }
        ConnectionsCmd::Add {
            integration,
            name,
            owner,
            template,
            identity_label,
            values,
        } => {
            let mut body = json!({
                "integration": integration,
                "name": name,
                "template": template,
                "values": parse_values(&values)?,
            });
            if let Some(owner) = owner {
                body["owner"] = json!(owner);
            }
            if let Some(label) = identity_label {
                body["identity_label"] = json!(label);
            }
            let created =
                daemon::post_json(&origin, "/api/connections", &body, token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&created)?);
        }
        ConnectionsCmd::Remove {
            owner,
            integration,
            name,
        } => {
            let path = format!("/api/connections/{owner}/{integration}/{name}");
            daemon::delete_json(&origin, &path, token.as_deref()).await?;
            println!("removed {owner}/{integration}/{name}");
        }
        ConnectionsCmd::Refresh {
            owner,
            integration,
            name,
        } => {
            let path = format!("/api/connections/{owner}/{integration}/{name}/refresh");
            let body = daemon::post_json(&origin, &path, &json!({}), token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
    }
    Ok(())
}

async fn cmd_balance(dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    let body = daemon::get_json(&origin, "/api/balance", token.as_deref()).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn cmd_team_tool(
    dir: Option<&Path>,
    cmd: TeamToolCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let token = daemon::bearer_token(dir);
    match cmd {
        TeamToolCmd::List => {
            let body = daemon::get_json(&origin, "/api/team-tools", token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&body)?);
        }
        TeamToolCmd::Add {
            name,
            provider,
            base_url,
            secret,
        } => {
            let body = json!({
                "name": name,
                "provider": provider,
                "base_url": base_url,
                "secret": secret,
            });
            let created =
                daemon::post_json(&origin, "/api/team-tools", &body, token.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&created)?);
        }
    }
    Ok(())
}
