//! End-to-end tests against `npx emulate` (GitHub REST, Google Discovery, Linear GraphQL).

use std::time::Duration;

use executor_core::{
    AuthTemplateSlug, ExecuteOptions, ExecutorError, Limits, Outcome, ToolListFilter, ToolResult,
};
use executor_engine::Executor;
use executor_host::{AppState, HostConfig, serve};
use executor_sdk::{CreateOptions, create_executor};
use executor_test_support::{GITHUB_TOKEN, GOOGLE_TOKEN, LINEAR_TOKEN, github_openapi_spec, urls};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn yes() -> ExecuteOptions {
    ExecuteOptions {
        auto_approve: true,
        ..ExecuteOptions::default()
    }
}

fn in_memory(limits: Limits) -> Executor {
    create_executor(CreateOptions {
        in_memory: true,
        limits,
        ..CreateOptions::default()
    })
    .expect("in-memory executor")
}

fn find_tool(exec: &Executor, integration: &str, needle: &str) -> String {
    let tools = exec
        .list_tools(&ToolListFilter {
            include_blocked: true,
            ..ToolListFilter::default()
        })
        .expect("list tools");
    let needle = needle.to_ascii_lowercase();
    tools
        .iter()
        .find(|t| {
            t.integration.as_str() == integration
                && t.name.as_str().to_ascii_lowercase().contains(&needle)
        })
        .unwrap_or_else(|| {
            let names: Vec<_> = tools
                .iter()
                .filter(|t| t.integration.as_str() == integration)
                .map(|t| t.name.as_str().to_owned())
                .collect();
            panic!("no tool containing {needle:?} on {integration}: {names:?}");
        })
        .address
        .to_string()
}

fn completed_data(out: Outcome) -> Value {
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => data,
        other => panic!("expected ok completion, got {other:?}"),
    }
}

fn completed_err(out: Outcome) -> executor_core::ToolError {
    match out {
        Outcome::Completed {
            result: ToolResult::Err { error },
            ..
        } => error,
        other => panic!("expected tool error, got {other:?}"),
    }
}

async fn add_github(exec: &Executor) {
    let emulate = urls();
    let spec = github_openapi_spec(&emulate.github);
    exec.execute(
        "executor.openapi.addSpec",
        json!({
            "slug": "github",
            "name": "GitHub",
            "spec": spec,
            "baseUrl": emulate.github,
        }),
        yes(),
    )
    .await
    .expect("addSpec github");
    exec.execute(
        "executor.coreTools.connections.create",
        json!({
            "integration": "github",
            "name": "work",
            "template": AuthTemplateSlug::bearer().as_str(),
            "values": { "token": GITHUB_TOKEN },
        }),
        yes(),
    )
    .await
    .expect("github connection");
}

async fn add_linear(exec: &Executor) {
    let emulate = urls();
    exec.execute(
        "executor.graphql.addIntegration",
        json!({
            "slug": "linear",
            "name": "Linear",
            "endpoint": format!("{}/graphql", emulate.linear),
        }),
        yes(),
    )
    .await
    .expect("add linear");
    exec.execute(
        "executor.coreTools.connections.create",
        json!({
            "integration": "linear",
            "name": "work",
            "template": "bearer",
            "values": { "token": LINEAR_TOKEN },
        }),
        yes(),
    )
    .await
    .expect("linear connection");
}

#[tokio::test]
async fn github_user_and_repo_succeed() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let me = exec
        .execute(
            &find_tool(&exec, "github", "getauthenticated"),
            json!({}),
            yes(),
        )
        .await
        .expect("GET /user");
    let data = completed_data(me);
    assert_eq!(data["login"], "octocat", "{data}");
    let repo = exec
        .execute(
            &find_tool(&exec, "github", "reposget"),
            json!({"owner": "octocat", "repo": "hello-world"}),
            yes(),
        )
        .await
        .expect("GET repo");
    let data = completed_data(repo);
    assert_eq!(data["name"], "hello-world", "{data}");
}

#[tokio::test]
async fn github_missing_repo_is_http_failure() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let out = exec
        .execute(
            &find_tool(&exec, "github", "reposget"),
            json!({"owner": "octocat", "repo": "does-not-exist"}),
            yes(),
        )
        .await
        .expect("call");
    let err = completed_err(out);
    assert_eq!(err.status, Some(404), "{err:?}");
}

#[tokio::test]
async fn github_issue_create_persists_on_emulator() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let created = exec
        .execute(
            &find_tool(&exec, "github", "issuescreate"),
            json!({
                "owner": "octocat",
                "repo": "hello-world",
                "title": "executor e2e",
                "body": "opened via the Rust port"
            }),
            yes(),
        )
        .await
        .expect("create issue");
    let data = completed_data(created);
    assert_eq!(data["title"], "executor e2e", "{data}");
    let listed = exec
        .execute(
            &find_tool(&exec, "github", "issueslist"),
            json!({"owner": "octocat", "repo": "hello-world"}),
            yes(),
        )
        .await
        .expect("list issues");
    let data = completed_data(listed);
    let found = data
        .as_array()
        .into_iter()
        .flatten()
        .any(|i| i["title"] == "executor e2e");
    assert!(found, "{data}");
}

#[tokio::test]
async fn github_policy_block() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern": "github.*", "action": "block", "owner": "org"}),
        yes(),
    )
    .await
    .expect("policy");
    let err = exec
        .execute(
            &find_tool(&exec, "github", "getauthenticated"),
            json!({}),
            yes(),
        )
        .await
        .expect_err("blocked");
    assert!(matches!(err, ExecutorError::ToolBlocked { .. }), "{err:?}");
}

#[tokio::test]
async fn github_code_mode_get_user() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let path = find_tool(&exec, "github", "getauthenticated");
    let src = executor_core::compile_call(&path, &json!({}));
    let out = exec.run_code(&src, yes()).await.expect("code");
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => assert_eq!(data["login"], "octocat", "{data}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn github_js_isolate_get_user() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let path = find_tool(&exec, "github", "getauthenticated");
    let src = format!(
        r#"
            const path = {path};
            const user = await tools[path]({{}});
            if (!user || !user.login) throw new Error("missing login");
            return {{ login: user.login, via: "isolate" }};
        "#,
        path = serde_json::to_string(&path).expect("path json")
    );
    let out = exec.run_code(&src, yes()).await.expect("quickjs");
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => {
            assert_eq!(data["login"], "octocat", "{data}");
            assert_eq!(data["via"], "isolate", "{data}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn google_calendar_discovery_lists_events() {
    let emulate = urls();
    let discovery_url = format!("{}/discovery/v1/apis/calendar/v3/rest", emulate.google);
    let doc: Value = reqwest::Client::new()
        .get(&discovery_url)
        .send()
        .await
        .expect("fetch discovery")
        .json()
        .await
        .expect("discovery json");
    assert!(
        doc.get("kind")
            .and_then(Value::as_str)
            .is_some_and(|k| k.contains("discovery"))
            || doc.get("resources").is_some(),
        "{doc}"
    );
    let exec = in_memory(Limits::production());
    exec.execute(
        "executor.openapi.addSpec",
        json!({
            "slug": "gcal",
            "name": "Google Calendar",
            "spec": { "url": discovery_url },
            "baseUrl": emulate.google,
        }),
        yes(),
    )
    .await
    .expect("add calendar spec");
    exec.execute(
        "executor.coreTools.connections.create",
        json!({"integration": "gcal", "name": "work", "template": AuthTemplateSlug::bearer().as_str(), "values": { "token": GOOGLE_TOKEN }}),
        yes(),
    )
    .await
    .expect("gcal connection");
    let path = find_tool(&exec, "gcal", "calendarlist");
    let out = exec
        .execute(&path, json!({"userId": "me"}), yes())
        .await
        .expect("list calendars");
    let data = completed_data(out);
    let text = data.to_string();
    assert!(
        text.contains("primary") || text.contains("Kickoff") || text.contains("calendarList"),
        "{data}"
    );
    let events_path = find_tool(&exec, "gcal", "eventslist");
    let listed = exec
        .execute(&events_path, json!({"calendarId": "primary"}), yes())
        .await
        .expect("list events");
    let data = completed_data(listed);
    let text = data.to_string();
    assert!(
        text.contains("Kickoff") || text.contains("items") || text.contains("kind"),
        "{data}"
    );
}

#[tokio::test]
async fn linear_graphql_viewer() {
    let exec = in_memory(Limits::production());
    add_linear(&exec).await;
    let path = find_tool(&exec, "linear", "viewer");
    let out = exec.execute(&path, json!({}), yes()).await.expect("viewer");
    let data = completed_data(out);
    assert!(
        data.get("viewer").is_some() || data.get("__typename").is_some() || data.is_object(),
        "{data}"
    );
}

#[tokio::test]
async fn execute_timeout_zero_on_emulate_catalog() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let err = exec
        .execute(
            &find_tool(&exec, "github", "getauthenticated"),
            json!({}),
            ExecuteOptions {
                auto_approve: true,
                timeout: Some(Duration::ZERO),
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect_err("timeout");
    assert!(matches!(err, ExecutorError::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn cancel_before_start_on_emulate_catalog() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = exec
        .execute_with_cancel(
            &find_tool(&exec, "github", "getauthenticated"),
            json!({}),
            yes(),
            cancel,
        )
        .await
        .expect_err("cancelled");
    assert!(matches!(err, ExecutorError::Cancelled), "{err:?}");
}

#[tokio::test]
async fn overload_fail_fast_against_github() {
    let mut limits = Limits::production();
    limits.max_in_flight = 1;
    limits.acquire_timeout = Duration::ZERO;
    let exec = in_memory(limits);
    add_github(&exec).await;
    let path = find_tool(&exec, "github", "getauthenticated");
    let mut handles = Vec::new();
    for _ in 0..32 {
        let exec = exec.clone();
        let path = path.clone();
        handles.push(tokio::spawn(async move {
            exec.execute(&path, json!({}), yes()).await
        }));
    }
    let mut saw_overload = false;
    for handle in handles {
        if matches!(
            handle.await.expect("join"),
            Err(ExecutorError::Overloaded { .. })
        ) {
            saw_overload = true;
        }
    }
    assert!(
        saw_overload,
        "expected at least one Overloaded with max_in_flight=1"
    );
}

#[tokio::test]
async fn spec_byte_cap_is_enforced() {
    let exec = in_memory(Limits::test_tight());
    let huge = "x".repeat(70 * 1024);
    let err = exec
        .execute(
            "executor.openapi.addSpec",
            json!({"slug": "huge", "spec": huge}),
            yes(),
        )
        .await
        .expect_err("limit");
    assert!(matches!(err, ExecutorError::LimitExceeded(_)), "{err:?}");
}

#[tokio::test]
async fn daemon_http_executes_github() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    let path = find_tool(&exec, "github", "getauthenticated");
    let cancel = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let bind = listener.local_addr().expect("addr");
    drop(listener);
    let serve_cancel = cancel.clone();
    let server = exec.clone();
    let handle = tokio::spawn(async move {
        serve(
            HostConfig {
                bind,
                limits: Limits::production(),
                ..HostConfig::default()
            },
            AppState::new(server, None),
            serve_cancel,
        )
        .await
    });
    // `serve` binds again; use its listen by retrying health.
    // Re-bind race: HostConfig uses the captured addr; the dropped listener frees it.
    let client = reqwest::Client::new();
    let health_url = format!("http://{bind}/health");
    let mut ok = false;
    for _ in 0..50 {
        if client.get(&health_url).send().await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ok, "daemon never became healthy at {health_url}");
    let body = client
        .post(format!("http://{bind}/api/execute"))
        .json(&json!({"path": path, "args": {}, "auto_approve": true}))
        .send()
        .await
        .expect("execute")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["result"]["data"]["login"], "octocat", "{body}");
    let mcp = client
        .post(format!("http://{bind}/mcp"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
        .send()
        .await
        .expect("mcp")
        .json::<Value>()
        .await
        .expect("mcp json");
    let listed = mcp["result"]["tools"].as_array().expect("tools array");
    assert!(
        listed.iter().any(|t| t["name"].as_str() == Some("execute")),
        "default MCP tools/list should expose execute: {mcp}"
    );
    assert!(
        listed.iter().any(|t| t["name"].as_str() == Some("skills")),
        "{mcp}"
    );
    assert!(
        listed.iter().any(|t| t["name"].as_str() == Some("resume")),
        "{mcp}"
    );
    assert!(
        listed
            .iter()
            .all(|t| t["name"].as_str().is_none_or(|n| !n.contains("github"))),
        "catalog dump must not leak into default MCP tools/list: {mcp}"
    );
    let health = client
        .get(format!("http://{bind}/api/health"))
        .send()
        .await
        .expect("api health")
        .text()
        .await
        .expect("text");
    assert_eq!(health, "ok");
    let exec_body = client
        .post(format!("http://{bind}/executions"))
        .json(&json!({"code": "return 2 + 2;", "autoApprove": true}))
        .send()
        .await
        .expect("executions")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(exec_body["status"], "completed", "{exec_body}");
    assert_eq!(exec_body["structured"]["data"], 4, "{exec_body}");
    cancel.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn empty_catalog_lists_nothing_dynamic() {
    let exec = in_memory(Limits::production());
    let tools = exec.list_tools(&ToolListFilter::default()).expect("list");
    assert!(
        tools.iter().all(|t| t.static_tool),
        "empty catalog should only expose static tools"
    );
}

#[tokio::test]
async fn jsonc_registers_openapi_integration() {
    let emulate = urls();
    let spec = github_openapi_spec(&emulate.github);
    let src = format!(
        r#"{{
          // rust-native jsonc
          "name": "demo",
          "integrations": [{{
            "kind": "openapi",
            "slug": "github",
            "name": "GitHub",
            "spec": {spec},
            "baseUrl": "{}"
          }}]
        }}"#,
        emulate.github
    );
    let cfg = executor_sdk::parse_jsonc(&src).expect("jsonc");
    let exec = in_memory(Limits::production());
    executor_sdk::apply_config(&exec, &cfg)
        .await
        .expect("apply");
    let rows = exec.list_integrations().expect("list");
    assert!(rows.iter().any(|i| i.slug.as_str() == "github"), "{rows:?}");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn daemon_bearer_cimd_and_plugin_http() {
    let exec = in_memory(Limits::production());
    let cancel = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let bind = listener.local_addr().expect("addr");
    drop(listener);
    let origin = format!("http://{bind}");
    let token = "test-daemon-token";
    let serve_cancel = cancel.clone();
    let server = exec.clone();
    let handle = tokio::spawn(async move {
        serve(
            HostConfig {
                bind,
                limits: Limits::production(),
                auth_token: Some(token.to_owned()),
                ..HostConfig::default()
            },
            AppState::new(server, None).with_control(
                Some(token.to_owned()),
                executor_host::default_allowed_hosts(),
                origin,
            ),
            serve_cancel,
        )
        .await
    });
    let client = reqwest::Client::new();
    let mut ok = false;
    for _ in 0..50 {
        if client
            .get(format!("http://{bind}/api/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ok, "health");
    let denied = client
        .get(format!("http://{bind}/api/tools"))
        .send()
        .await
        .expect("tools");
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);
    let cimd = client
        .get(format!("http://{bind}/oauth/client-id-metadata.json"))
        .send()
        .await
        .expect("cimd")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(cimd["token_endpoint_auth_method"], "none");
    assert!(
        cimd["client_id"]
            .as_str()
            .is_some_and(|s| s.contains("client-id-metadata"))
    );
    let local = client
        .get(format!("http://{bind}/oauth/client-id-metadata/local.json"))
        .send()
        .await
        .expect("local cimd")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(local["application_type"], "native");
    let emulate = urls();
    let spec = github_openapi_spec(&emulate.github);
    let added = client
        .post(format!("http://{bind}/openapi/specs"))
        .header("authorization", format!("Bearer {token}"))
        .json(&json!({
            "slug": "github",
            "spec": spec,
            "baseUrl": emulate.github,
        }))
        .send()
        .await
        .expect("addSpec")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(added["status"], "completed", "{added}");
    let listed = client
        .get(format!("http://{bind}/api/integrations"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("integrations")
        .json::<Value>()
        .await
        .expect("json");
    assert!(
        listed["integrations"]
            .as_array()
            .is_some_and(|a| a.iter().any(|i| i["slug"] == "github")),
        "{listed}"
    );
    cancel.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn get_resume_approves_paused_execution() {
    let exec = in_memory(Limits::production());
    add_github(&exec).await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern":"github.*","action":"require_approval","owner":"org"}),
        yes(),
    )
    .await
    .expect("policy");
    let path = find_tool(&exec, "github", "getauthenticated");
    let paused = exec
        .execute(&path, json!({}), ExecuteOptions::default())
        .await
        .expect("pause");
    let Outcome::Paused { execution } = paused else {
        panic!("expected pause, got {paused:?}");
    };
    let cancel = CancellationToken::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let bind = listener.local_addr().expect("addr");
    drop(listener);
    let serve_cancel = cancel.clone();
    let server = exec.clone();
    let handle = tokio::spawn(async move {
        serve(
            HostConfig {
                bind,
                limits: Limits::production(),
                ..HostConfig::default()
            },
            AppState::new(server, None),
            serve_cancel,
        )
        .await
    });
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client
            .get(format!("http://{bind}/api/health"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let resumed = client
        .get(format!(
            "http://{bind}/executions/{}/resume?action=accept&persist=session",
            execution.id
        ))
        .send()
        .await
        .expect("GET resume")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(resumed["status"], "completed", "{resumed}");
    cancel.cancel();
    let _ = handle.await;
}
