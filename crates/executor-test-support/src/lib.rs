//! Attach to [`npx emulate`](https://github.com/vercel-labs/emulate) for integration tests.
//!
//! This crate does not vendor emulate. Tests start the published CLI (or reuse a
//! process already bound to the well-known ports / `*_EMULATOR_URL` env vars).

use std::fs::OpenOptions;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// GitHub REST emulator.
pub const GITHUB_TOKEN: &str = "gh_test";
/// Linear GraphQL emulator personal API key.
pub const LINEAR_TOKEN: &str = "lin_test_admin";

/// Default base port for `npx emulate --service github,google,linear`.
pub const BASE_PORT: u16 = 18_400;

static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static URLS: OnceLock<EmulateUrls> = OnceLock::new();

/// Base URLs of the three services this workspace's tests drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmulateUrls {
    /// GitHub REST (`GET /user`, repos, issues).
    pub github: String,
    /// Google (Calendar discovery, Gmail, Drive).
    pub google: String,
    /// Linear GraphQL (`POST /graphql`).
    pub linear: String,
}

impl EmulateUrls {
    fn local(base: u16) -> Self {
        Self {
            github: format!("http://127.0.0.1:{base}"),
            google: format!("http://127.0.0.1:{}", base + 1),
            linear: format!("http://127.0.0.1:{}", base + 2),
        }
    }
}

/// Start `npx emulate` if needed and return the service URLs.
///
/// # Panics
///
/// When Node/`npx` is missing, the seed file is missing, or the emulator never
/// accepts TCP on the expected ports.
#[must_use]
pub fn urls() -> EmulateUrls {
    URLS.get_or_init(boot).clone()
}

fn boot() -> EmulateUrls {
    if let Some(from_env) = from_env() {
        wait_until_ready(&from_env, Duration::from_secs(120));
        return from_env;
    }
    let urls = EmulateUrls::local(BASE_PORT);
    if all_ready(&urls) {
        return urls;
    }
    spawn_emulate();
    wait_until_ready(&urls, Duration::from_secs(180));
    urls
}

fn from_env() -> Option<EmulateUrls> {
    let github = std::env::var("GITHUB_EMULATOR_URL").ok()?;
    let google = std::env::var("GOOGLE_EMULATOR_URL").unwrap_or_else(|_| sibling(&github, 1));
    let linear = std::env::var("LINEAR_EMULATOR_URL").unwrap_or_else(|_| sibling(&github, 2));
    Some(EmulateUrls {
        github,
        google,
        linear,
    })
}

fn sibling(url: &str, offset: i32) -> String {
    let trimmed = url.trim_end_matches('/');
    let Some((prefix, port)) = trimmed.rsplit_once(':') else {
        return trimmed.to_owned();
    };
    let parsed: i32 = port.parse().unwrap_or_else(|_| i32::from(BASE_PORT));
    format!("{prefix}:{}", parsed + offset)
}

fn spawn_emulate() {
    let root = workspace_root();
    let seed = root.join("emulate.config.yaml");
    assert!(
        seed.is_file(),
        "missing {} — required to seed emulate",
        seed.display()
    );
    let lock_path = std::env::temp_dir().join("executor-emulate-18400.lock");
    let we_own = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .is_ok();
    if !we_own {
        return;
    }
    let child = Command::new("npx")
        .current_dir(&root)
        .args([
            "--yes",
            "emulate@0.11.2",
            "--service",
            "github,google,linear",
            "--seed",
            seed.to_str().unwrap_or("emulate.config.yaml"),
            "--port",
            &BASE_PORT.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_file(&lock_path);
            panic!("npx emulate failed to start ({e}). Install Node.js 22+ so tests can invoke the emulate CLI.");
        });
    CHILD
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace(child);
}

fn wait_until_ready(urls: &EmulateUrls, budget: Duration) {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if all_ready(urls) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    panic!(
        "emulate did not become ready at {}, {}, {} — is `npx emulate` installed?",
        urls.github, urls.google, urls.linear
    );
}

fn all_ready(urls: &EmulateUrls) -> bool {
    port_open(&urls.github) && port_open(&urls.google) && port_open(&urls.linear)
}

fn port_open(url: &str) -> bool {
    let Some(addr) = url_host_port(url) else {
        return false;
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn url_host_port(url: &str) -> Option<std::net::SocketAddr> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let hostport = rest.split('/').next()?;
    hostport.parse().ok().or_else(|| {
        let (host, port) = hostport.split_once(':')?;
        let port: u16 = port.parse().ok()?;
        format!("{host}:{port}").parse().ok()
    })
}

fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..8 {
        if dir.join("emulate.config.yaml").is_file() && dir.join("Cargo.toml").is_file() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    panic!(
        "emulate.config.yaml not found above {}",
        env!("CARGO_MANIFEST_DIR")
    );
}

/// Minimal GitHub `OpenAPI` 3 document aimed at the emulator REST surface.
#[must_use]
pub fn github_openapi_spec(base_url: &str) -> Value {
    json!({
        "openapi": "3.0.0",
        "info": {"title": "GitHub emulator", "version": "1"},
        "servers": [{"url": base_url}],
        "paths": {
            "/user": {
                "get": {
                    "operationId": "usersGetAuthenticated",
                    "tags": ["users"],
                    "summary": "Get the authenticated user",
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/users/{username}": {
                "get": {
                    "operationId": "usersGetByUsername",
                    "tags": ["users"],
                    "summary": "Get a user",
                    "parameters": [{
                        "name": "username",
                        "in": "path",
                        "required": true,
                        "schema": {"type": "string"}
                    }],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/repos/{owner}/{repo}": {
                "get": {
                    "operationId": "reposGet",
                    "tags": ["repos"],
                    "summary": "Get a repository",
                    "parameters": [
                        {"name": "owner", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "repo", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/repos/{owner}/{repo}/issues": {
                "get": {
                    "operationId": "issuesListForRepo",
                    "tags": ["issues"],
                    "summary": "List issues",
                    "parameters": [
                        {"name": "owner", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "repo", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {"200": {"description": "ok"}}
                },
                "post": {
                    "operationId": "issuesCreate",
                    "tags": ["issues"],
                    "summary": "Create an issue",
                    "parameters": [
                        {"name": "owner", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "repo", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["title"],
                                    "properties": {
                                        "title": {"type": "string"},
                                        "body": {"type": "string"}
                                    }
                                }
                            }
                        }
                    },
                    "responses": {"201": {"description": "created"}}
                }
            }
        }
    })
}

/// Absolute path to the repo-root emulate seed file.
#[must_use]
pub fn seed_path() -> PathBuf {
    workspace_root().join("emulate.config.yaml")
}

/// Whether `path` exists. Used by CLI tests to fail with a clear message.
#[must_use]
pub fn seed_exists() -> bool {
    Path::new(&seed_path()).is_file()
}
