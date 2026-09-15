//! `executor` CLI — catalog, call, resume, MCP, daemon. No web UI.

#![allow(clippy::module_name_repetitions)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use executor_core::{
    ExecuteOptions, ExecutionId, IdempotencyKey, ResumeAction, ToolListFilter, resolve_invocation,
};
use executor_host::{AppState, DEFAULT_PORT, DEFAULT_SERVICE_PORT, HostConfig, serve, stdio_loop};
use executor_sdk::{CreateOptions, create_executor, create_executor_with_metrics, data_dir};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// Executor — integration catalog for agents.
#[derive(Parser, Debug)]
#[command(
    name = "executor",
    version,
    about = "Integration catalog for agents (no UI)"
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
    /// Invoke a tool (`executor call path.to.tool '{...json...}'`).
    Call {
        /// Path segments and optional trailing JSON / `@file.json`.
        #[arg(required = true)]
        path: Vec<String>,
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
    },
    /// Tool catalog.
    Tools {
        #[command(subcommand)]
        cmd: ToolsCmd,
    },
    /// MCP stdio host.
    Mcp,
    /// Foreground HTTP daemon (loopback).
    Serve {
        /// Bind port.
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
        /// Ignored: this port never daemonizes; always foreground.
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
    match cli.command {
        Commands::Call {
            path,
            yes,
            idempotency_key,
        } => cmd_call(cli.data_dir.as_deref(), path, yes, idempotency_key).await,
        Commands::Resume {
            execution_id,
            action,
        } => cmd_resume(cli.data_dir.as_deref(), execution_id, action).await,
        Commands::Tools { cmd } => cmd_tools(cli.data_dir.as_deref(), cmd),
        Commands::Mcp => {
            let exec = create_executor(CreateOptions {
                data_dir: cli.data_dir,
                ..CreateOptions::default()
            })?;
            stdio_loop(exec).await?;
            Ok(())
        }
        Commands::Serve { port } => daemon_run(cli.data_dir.as_deref(), port).await,
        Commands::Daemon { cmd } => match cmd {
            DaemonCmd::Run { port, .. } => daemon_run(cli.data_dir.as_deref(), port).await,
            DaemonCmd::Status => {
                daemon_status(cli.data_dir.as_deref());
                Ok(())
            }
            DaemonCmd::Stop => {
                daemon_stop(cli.data_dir.as_deref());
                Ok(())
            }
            DaemonCmd::Restart { port } => {
                daemon_stop(cli.data_dir.as_deref());
                daemon_run(cli.data_dir.as_deref(), port).await
            }
        },
        Commands::Install => install(cli.data_dir.as_deref()),
        Commands::Uninstall => uninstall(),
    }
}

async fn cmd_call(
    dir: Option<&Path>,
    path: Vec<String>,
    yes: bool,
    idempotency_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let invocation = resolve_invocation(&path)?;
    let exec = create_executor(CreateOptions {
        data_dir: dir.map(Path::to_path_buf),
        ..CreateOptions::default()
    })?;
    let key = idempotency_key.map(IdempotencyKey::new).transpose()?;
    let outcome = exec
        .execute(
            &invocation.path,
            Value::Object(invocation.args),
            ExecuteOptions {
                auto_approve: yes,
                timeout: None,
                idempotency_key: key,
            },
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&outcome.cli_json())?);
    Ok(())
}

async fn cmd_resume(
    dir: Option<&Path>,
    execution_id: String,
    action: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exec = create_executor(CreateOptions {
        data_dir: dir.map(Path::to_path_buf),
        ..CreateOptions::default()
    })?;
    let id = ExecutionId::new(execution_id)?;
    let action = match action.as_str() {
        "accept" => ResumeAction::Accept,
        "decline" => ResumeAction::Decline,
        "cancel" => ResumeAction::Cancel,
        other => return Err(format!("unknown action {other}").into()),
    };
    let outcome = exec.resume(&id, action).await?;
    println!("{}", serde_json::to_string_pretty(&outcome.cli_json())?);
    Ok(())
}

fn cmd_tools(
    dir: Option<&Path>,
    cmd: ToolsCmd,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let exec = create_executor(CreateOptions {
        data_dir: dir.map(Path::to_path_buf),
        ..CreateOptions::default()
    })?;
    match cmd {
        ToolsCmd::List => {
            let tools = exec.list_tools(&ToolListFilter::default())?;
            print_tools(&tools);
        }
        ToolsCmd::Search { query } => {
            let tools = exec.list_tools(&ToolListFilter {
                query: Some(query),
                ..ToolListFilter::default()
            })?;
            if tools.is_empty() {
                println!("(no matching tools)");
            } else {
                print_tools(&tools);
            }
        }
        ToolsCmd::Integrations => {
            let rows = exec.list_integrations()?;
            if rows.is_empty() {
                println!("(no integrations)");
            } else {
                for i in rows {
                    println!("{}\t{}\t{}", i.slug, i.kind, i.name);
                }
            }
        }
        ToolsCmd::Describe { path } => {
            let joined = path.join(".");
            let tool = exec.describe(&joined)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "path": tool.cli_path(),
                    "address": tool.address.to_string(),
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }))?
            );
        }
    }
    Ok(())
}

fn print_tools(tools: &[executor_core::Tool]) {
    for t in tools {
        println!("{}\t{}", t.cli_path(), t.description);
    }
}

async fn daemon_run(
    dir: Option<&Path>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let metrics = Arc::new(executor_core::AtomicMetrics::new());
    let exec = create_executor_with_metrics(
        CreateOptions {
            data_dir: dir.map(Path::to_path_buf),
            ..CreateOptions::default()
        },
        Arc::clone(&metrics) as Arc<dyn executor_core::Metrics>,
    )?;
    let cancel = exec.cancellation_token();
    let bind = SocketAddr::from(([127, 0, 0, 1], port));
    let pid_path = data_dir(dir).join("daemon.pid");
    write_pid(&pid_path)?;
    let ctrlc_task = ctrlc(cancel.clone());
    tracing::info!(%bind, "executor daemon listening");
    let result = serve(
        HostConfig {
            bind,
            limits: exec.limits().clone(),
        },
        AppState {
            executor: exec,
            metrics: Some(metrics),
        },
        cancel,
    )
    .await;
    let _ = std::fs::remove_file(pid_path);
    drop(ctrlc_task);
    result?;
    Ok(())
}

fn daemon_status(dir: Option<&Path>) {
    let pid_path = data_dir(dir).join("daemon.pid");
    match std::fs::read_to_string(&pid_path) {
        Ok(s) => {
            let pid = s.trim();
            println!("pid {pid} ({})", pid_path.display());
        }
        Err(_) => println!("daemon is not running"),
    }
}

fn daemon_stop(dir: Option<&Path>) {
    let pid_path = data_dir(dir).join("daemon.pid");
    if let Ok(s) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = s.trim().parse::<i32>() {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .status();
        }
        let _ = std::fs::remove_file(pid_path);
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

/// Keep `DEFAULT_SERVICE_PORT` referenced in help text.
#[allow(dead_code)]
const _SERVICE: u16 = DEFAULT_SERVICE_PORT;
