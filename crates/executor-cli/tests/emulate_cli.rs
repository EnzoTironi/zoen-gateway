//! CLI end-to-end against `npx emulate`.

use std::process::Command;

use executor_test_support::{GITHUB_TOKEN, github_openapi_spec, urls};
use serde_json::{Value, json};

const fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_executor")
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
    assert!(!stdout.to_ascii_lowercase().contains("web ui"));
    let (code, stdout, stderr) = run(dir.path(), &["tools", "integrations"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("(no integrations)"), "{stdout}");
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
        .find(|line| line.contains("github") && line.to_ascii_lowercase().contains("user"))
        .and_then(|line| line.split('\t').next())
        .expect("github user tool in `tools list`");
    let call = json!({}).to_string();
    let (code, stdout, stderr) = run(dir.path(), &["call", "--yes", tool_path, &call]);
    assert_eq!(code, 0, "GET /user via CLI failed: {stdout}{stderr}");
    let parsed: Value = serde_json::from_str(&stdout).unwrap_or_else(|_| json!({}));
    let login = parsed.pointer("/result/data/login");
    assert_eq!(login, Some(&json!("octocat")), "{stdout}");
}
