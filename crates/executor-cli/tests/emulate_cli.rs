//! CLI end-to-end against `npx emulate`.

use std::process::Command;

use executor_test_support::{GITHUB_TOKEN, github_openapi_spec, urls};
use serde_json::{Value, json};

const fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_executor")
}

fn stop(dir: &std::path::Path) {
    let _ = Command::new(bin())
        .env("EXECUTOR_DATA_DIR", dir)
        .args(["daemon", "stop"])
        .output();
}

fn run(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .env("EXECUTOR_DATA_DIR", dir)
        .args(args)
        .output()
        .expect("spawn executor");
    let code = out.status.code().unwrap_or(1);
    (
        code,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_and_empty_integrations() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let (code, stdout, stderr) = run(dir.path(), &["--help"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("call"), "{stdout}");
    assert!(stdout.contains("login"), "{stdout}");
    assert!(stdout.contains("server"), "{stdout}");
    assert!(
        !stdout
            .lines()
            .any(|line| line.trim_start().starts_with("web ")),
        "web subcommand must stay unported: {stdout}"
    );
    let (code, stdout, stderr) = run(dir.path(), &["tools", "integrations"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("(no integrations)"), "{stdout}");
    stop(dir.path());
}

#[test]
fn call_github_via_cli() {
    let emulate = urls();
    let dir = tempfile::tempdir().expect("tmpdir");
    let spec = github_openapi_spec(&emulate.github);
    let add = json!({
        "slug": "github",
        "spec": spec,
        "baseUrl": emulate.github,
    });
    let spec_file = dir.path().join("add.json");
    std::fs::write(&spec_file, add.to_string()).expect("write add.json");
    let add_arg = format!("@{}", spec_file.display());
    let (code, stdout, stderr) = run(
        dir.path(),
        &["call", "--yes", "executor.openapi.addSpec", &add_arg],
    );
    assert_eq!(code, 0, "addSpec failed: {stdout}{stderr}");

    let conn = json!({
        "integration": "github",
        "name": "work",
        "template": "bearer",
        "values": { "token": GITHUB_TOKEN },
    })
    .to_string();
    let (code, stdout, stderr) = run(
        dir.path(),
        &[
            "call",
            "--yes",
            "executor.coreTools.connections.create",
            &conn,
        ],
    );
    assert_eq!(code, 0, "connection failed: {stdout}{stderr}");

    let (code, stdout, stderr) = run(dir.path(), &["tools", "list"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("github"), "{stdout}");
    let tool_path = stdout
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("authenticated"))
        .and_then(|line| line.split('\t').next())
        .expect("github authenticated-user tool in `tools list`");
    let call = json!({}).to_string();
    let (code, stdout, stderr) = run(dir.path(), &["call", "--yes", tool_path, &call]);
    assert_eq!(code, 0, "GET /user via CLI failed: {stdout}{stderr}");
    let parsed: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| json!({}));
    let login = parsed
        .pointer("/structured/data/login")
        .or_else(|| parsed.pointer("/result/data/login"));
    assert_eq!(login, Some(&json!("octocat")), "{stdout}");
    stop(dir.path());
}

#[test]
fn call_js_isolate_via_cli() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let out = Command::new(bin())
        .env("EXECUTOR_DATA_DIR", dir.path())
        .env("EXECUTOR_KERNEL", "js")
        .args(["call", "--yes", "--code", "return 1 + 2;"])
        .output()
        .expect("spawn executor");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    let parsed: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| json!({}));
    assert_eq!(
        parsed
            .pointer("/structured/data")
            .or_else(|| parsed.pointer("/result/data")),
        Some(&json!(3)),
        "{stdout}"
    );
    stop(dir.path());
}

#[test]
fn server_profiles_and_whoami() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let (code, stdout, stderr) = run(
        dir.path(),
        &[
            "server",
            "add",
            "cloud",
            "--origin",
            "https://example.test",
            "--default",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, stderr) = run(dir.path(), &["server", "list"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("cloud"), "{stdout}");
    let (code, stdout, stderr) = run(dir.path(), &["whoami"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("cloud"), "{stdout}");
    assert!(stdout.contains("example.test"), "{stdout}");
    let (code, _, stderr) = run(dir.path(), &["login", "--help"]);
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn call_help_browses_namespaces() {
    let emulate = urls();
    let dir = tempfile::tempdir().expect("tmpdir");
    let spec = github_openapi_spec(&emulate.github);
    let add = json!({
        "slug": "github",
        "spec": spec,
        "baseUrl": emulate.github,
    });
    let spec_file = dir.path().join("add.json");
    std::fs::write(&spec_file, add.to_string()).expect("write");
    let add_arg = format!("@{}", spec_file.display());
    let (code, stdout, stderr) = run(
        dir.path(),
        &["call", "--yes", "executor.openapi.addSpec", &add_arg],
    );
    assert_eq!(code, 0, "addSpec failed: {stdout}{stderr}");
    let conn = json!({
        "integration": "github",
        "name": "work",
        "template": "none",
    })
    .to_string();
    let (code, stdout, stderr) = run(
        dir.path(),
        &[
            "call",
            "--yes",
            "executor.coreTools.connections.create",
            &conn,
        ],
    );
    assert_eq!(code, 0, "connection failed: {stdout}{stderr}");
    let (code, stdout, stderr) = run(dir.path(), &["call", "--help"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Browse:"), "{stdout}");
    assert!(
        stdout.contains("github") || stdout.contains("tools"),
        "{stdout}"
    );
    let (code, stdout, stderr) = run(dir.path(), &["call", "--help", "github"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(
        stdout.contains("prefix:") || stdout.contains("org") || stdout.contains("github"),
        "{stdout}"
    );
    stop(dir.path());
}

#[test]
fn install_boot_writes_unit() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let out = Command::new(bin())
        .env("EXECUTOR_DATA_DIR", dir.path())
        .env("HOME", dir.path())
        .args(["install", "--boot"])
        .output()
        .expect("install");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code().unwrap_or(1), 0, "{stdout}{stderr}");
    let unit = dir.path().join(".config/systemd/user/executor.service");
    let text = std::fs::read_to_string(&unit).expect("unit");
    assert!(text.contains("--foreground"), "{text}");
    assert!(text.contains("--hostname"), "{text}");
    assert!(
        stdout.contains("linger") || stdout.contains("wrote"),
        "{stdout}"
    );
}
