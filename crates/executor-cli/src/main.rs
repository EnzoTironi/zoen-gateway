//! `executor` CLI — catalog, call, resume, MCP, daemon, login. No web UI.

#![allow(clippy::module_name_repetitions)]

mod daemon;
mod login;
mod mcp_bridge;
mod profiles;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use executor_core::{compile_call, resolve_invocation, unix_now_ms};
use executor_host::{AppState, DEFAULT_PORT, DEFAULT_SERVICE_PORT, HostConfig, serve};
use executor_sdk::{CreateOptions, create_executor_with_metrics, data_dir};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// Executor — integration catalog for agents.
#[derive(Parser, Debug)]
#[command(
    name = "executor",
    version,
    about = "Integration catalog for agents (CLI only)"
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
    Call {
        /// Path segments and optional trailing JSON / `@file.json`.
        path: Vec<String>,
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
    /// Write a systemd user unit (Linux).
    Install,
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
    /// Run the daemon (foreground; writes a pid file).
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        foreground: bool,
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
enum ServiceCmd {
    /// Alias of `install`.
    Install,
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

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = cli.data_dir.clone();
    match cli.command {
        Commands::Call {
            path,
            code,
            yes,
            idempotency_key,
        } => cmd_call(dir.as_deref(), path, code, yes, idempotency_key).await,
        Commands::Resume {
            execution_id,
            action,
            content,
        } => cmd_resume(dir.as_deref(), execution_id, action, content).await,
        Commands::Tools { cmd } => cmd_tools(dir.as_deref(), cmd).await,
        Commands::Mcp {
            mode,
            elicitation_mode,
            search_tools,
        } => {
            let origin = daemon::ensure_daemon(dir.as_deref()).await?;
            let mut q = format!("?mode={mode}&elicitation_mode={elicitation_mode}");
            if search_tools {
                q.push_str("&search_tools=true");
            }
            mcp_bridge::run(&origin, &q).await?;
            Ok(())
        }
        Commands::Serve { port } => daemon_run(dir.as_deref(), port).await,
        Commands::Daemon { cmd } => daemon_cmd(dir.as_deref(), cmd).await,
        Commands::Install
        | Commands::Service {
            cmd: ServiceCmd::Install,
        } => install(dir.as_deref()),
        Commands::Uninstall
        | Commands::Service {
            cmd: ServiceCmd::Uninstall,
        } => uninstall(),
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
            daemon_run(dir.as_deref(), port).await
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
    }
}

async fn daemon_cmd(
    dir: Option<&Path>,
    cmd: DaemonCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cmd {
        DaemonCmd::Run { port, .. } => daemon_run(dir, port).await,
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
            daemon_run(dir, port).await
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
    let outcome = daemon::post_json(&origin, "/executions", &body).await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

async fn cmd_resume(
    dir: Option<&Path>,
    execution_id: String,
    action: String,
    content: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    let mut body = json!({ "action": action });
    if let Some(raw) = content {
        body["content"] = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    }
    let path = format!("/executions/{execution_id}/resume");
    let outcome = daemon::post_json(&origin, &path, &body).await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

async fn cmd_tools(
    dir: Option<&Path>,
    cmd: ToolsCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let origin = daemon::ensure_daemon(dir).await?;
    match cmd {
        ToolsCmd::List => {
            let body = daemon::get_json(&origin, "/api/tools").await?;
            print_tool_rows(&body);
        }
        ToolsCmd::Search { query } => {
            let q = urlencoding_query(&query);
            let body = daemon::get_json(&origin, &format!("/api/tools?q={q}")).await?;
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
            let body = daemon::get_json(&origin, "/api/integrations").await?;
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
            let body = daemon::get_json(&origin, &format!("/api/tools?q={q}")).await?;
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

fn urlencoding_query(value: &str) -> String {
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

fn bind_addr(port: u16) -> SocketAddr {
    let host = std::env::var("EXECUTOR_BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let ip: IpAddr = host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    SocketAddr::from((ip, port))
}

async fn daemon_run(
    dir: Option<&Path>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let metrics = Arc::new(executor_core::AtomicMetrics::new());
    let (sink, sentry) =
        executor_host::attach_sentry(Arc::clone(&metrics) as Arc<dyn executor_core::Metrics>);
    let exec = create_executor_with_metrics(
        CreateOptions {
            data_dir: dir.map(Path::to_path_buf),
            ..CreateOptions::default()
        },
        sink,
    )?;
    let cancel = exec.cancellation_token();
    let bind = bind_addr(port);
    let pid_path = data_dir(dir).join("daemon.pid");
    write_pid(&pid_path)?;
    let ctrlc_task = ctrlc(cancel.clone());
    tracing::info!(%bind, "executor daemon listening");
    let pointer = daemon::DaemonPointer {
        origin: format!("http://127.0.0.1:{}", bind.port()),
        pid: std::process::id(),
    };
    let _ = std::fs::create_dir_all(data_dir(dir));
    let _ = std::fs::write(
        daemon::pointer_path(dir),
        serde_json::to_vec_pretty(&pointer).unwrap_or_default(),
    );
    let result = serve(
        HostConfig {
            bind,
            limits: exec.limits().clone(),
        },
        AppState::new(exec, Some(metrics)),
        cancel,
    )
    .await;
    let _ = std::fs::remove_file(pid_path);
    drop(ctrlc_task);
    result?;
    drop(sentry);
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

fn install(dir: Option<&Path>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exe = std::env::current_exe()?;
    let data = data_dir(dir);
    let unit = format!(
        "[Unit]\nDescription=Executor daemon\n[Service]\nExecStart={} daemon run --port {DEFAULT_SERVICE_PORT}\nEnvironment=EXECUTOR_DATA_DIR={}\nRestart=on-failure\n[Install]\nWantedBy=default.target\n",
        exe.display(),
        data.display()
    );
    let path = systemd_unit_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, unit)?;
    println!("wrote {}", path.display());
    println!("enable with: systemctl --user enable --now executor.service");
    Ok(())
}

fn uninstall() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = systemd_unit_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("not installed"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn systemd_unit_path() -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".config/systemd/user/executor.service"))
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
