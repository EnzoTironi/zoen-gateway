//! Failure, cancellation, timeout, and overload coverage — not only the happy path.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use executor_core::{
    AuthMethod, ConnectionInput, ConnectionName, Detection, ExecuteOptions, ExecutorError,
    HealthCheckCtx, IdempotencyKey, IntegrationConfig, IntegrationPlugin, IntegrationSlug,
    InvokeCtx, Limits, Outcome, PluginError, PluginId, PolicyAction, RegisterIntegration,
    ResolveToolsCtx, ResolvedTools, ResumeAction, ResumeRequest, ToolDef, ToolName, ToolResult,
};
use serde_json::json;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::Executor;

struct EchoPlugin;

#[async_trait]
impl IntegrationPlugin for EchoPlugin {
    fn id(&self) -> PluginId {
        PluginId::new("echo").expect("echo")
    }

    fn detect(&self, candidate: &str) -> Option<Detection> {
        candidate.starts_with("echo:").then(|| Detection {
            kind: PluginId::new("echo").expect("echo"),
            confidence: executor_core::DetectionConfidence::High,
            endpoint: candidate.to_owned(),
            name: "Echo".into(),
            slug: "echo".into(),
        })
    }

    fn describe_auth(&self, _: &IntegrationConfig) -> Vec<AuthMethod> {
        vec![AuthMethod::none()]
    }

    async fn resolve_tools(&self, _: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError> {
        Ok(ResolvedTools {
            tools: vec![ToolDef {
                name: ToolName::new("ping").expect("ping"),
                description: "echo args".into(),
                input_schema: Some(json!({"type":"object","properties":{"n":{"type":"integer"}}})),
                output_schema: None,
                annotations: None,
                plugin_meta: None,
            }],
            definitions: None,
            incomplete: false,
            incomplete_reason: None,
        })
    }

    async fn invoke(&self, ctx: InvokeCtx<'_>) -> Result<ToolResult, PluginError> {
        Ok(ToolResult::ok(ctx.args.clone()))
    }

    async fn check_health(
        &self,
        _: HealthCheckCtx<'_>,
    ) -> Result<executor_core::HealthVerdict, PluginError> {
        Ok(executor_core::HealthVerdict::Unknown)
    }
}

struct SleepPlugin {
    delay: Duration,
    started: Arc<Notify>,
}

#[async_trait]
impl IntegrationPlugin for SleepPlugin {
    fn id(&self) -> PluginId {
        PluginId::new("sleep").expect("sleep")
    }

    fn detect(&self, _: &str) -> Option<Detection> {
        None
    }

    fn describe_auth(&self, _: &IntegrationConfig) -> Vec<AuthMethod> {
        vec![AuthMethod::none()]
    }

    async fn resolve_tools(&self, _: ResolveToolsCtx<'_>) -> Result<ResolvedTools, PluginError> {
        Ok(ResolvedTools {
            tools: vec![ToolDef {
                name: ToolName::new("wait").expect("wait"),
                description: "sleep".into(),
                input_schema: None,
                output_schema: None,
                annotations: None,
                plugin_meta: None,
            }],
            definitions: None,
            incomplete: false,
            incomplete_reason: None,
        })
    }

    async fn invoke(&self, _: InvokeCtx<'_>) -> Result<ToolResult, PluginError> {
        self.started.notify_waiters();
        tokio::time::sleep(self.delay).await;
        Ok(ToolResult::ok(json!({"slept": true})))
    }

    async fn check_health(
        &self,
        _: HealthCheckCtx<'_>,
    ) -> Result<executor_core::HealthVerdict, PluginError> {
        Ok(executor_core::HealthVerdict::Unknown)
    }
}

async fn wired(plugin: Arc<dyn IntegrationPlugin>, limits: Limits, slug: &str) -> Executor {
    let exec = Executor::builder().plugin(plugin).limits(limits).build();
    exec.register_integration(
        RegisterIntegration {
            slug: IntegrationSlug::new(slug).expect("slug"),
            name: Some(slug.into()),
            description: slug.into(),
            config: json!({}),
            can_remove: true,
            can_refresh: true,
        },
        PluginId::new(slug).expect("kind"),
    )
    .expect("register");
    exec.create_connection(ConnectionInput {
        owner: executor_core::Owner::Org,
        name: ConnectionName::new("work").expect("name"),
        integration: IntegrationSlug::new(slug).expect("slug"),
        template: executor_core::AuthTemplateSlug::none(),
        identity_label: None,
        description: None,
        values: std::collections::BTreeMap::new(),
        refs: std::collections::BTreeMap::new(),
    })
    .await
    .expect("connection");
    exec
}

#[tokio::test]
async fn execute_echo_happy_path() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let out = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 1}),
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect("execute");
    match out {
        Outcome::Completed { result, .. } => match result {
            ToolResult::Ok { data, .. } => assert_eq!(data["n"], 1),
            ToolResult::Err { error } => panic!("{error:?}"),
        },
        Outcome::Paused { .. } => panic!("paused"),
    }
}

#[tokio::test]
async fn overload_fail_fast() {
    let started = Arc::new(Notify::new());
    let exec = wired(
        Arc::new(SleepPlugin {
            delay: Duration::from_secs(2),
            started: Arc::clone(&started),
        }),
        Limits::test_tight(),
        "sleep",
    )
    .await;
    let exec_bg = exec.clone();
    let handle = tokio::spawn(async move {
        exec_bg
            .execute(
                "tools.sleep.org.work.wait",
                json!({}),
                ExecuteOptions {
                    auto_approve: true,
                    timeout: Some(Duration::from_secs(2)),
                    ..ExecuteOptions::default()
                },
            )
            .await
    });
    started.notified().await;
    let err = exec
        .execute(
            "tools.sleep.org.work.wait",
            json!({}),
            ExecuteOptions {
                auto_approve: true,
                timeout: Some(Duration::from_secs(2)),
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect_err("overload");
    assert!(matches!(err, ExecutorError::Overloaded { .. }), "{err:?}");
    handle.abort();
}

#[tokio::test]
async fn timeout_on_slow_plugin() {
    let exec = wired(
        Arc::new(SleepPlugin {
            delay: Duration::from_secs(2),
            started: Arc::new(Notify::new()),
        }),
        Limits::test_tight(),
        "sleep",
    )
    .await;
    let err = exec
        .execute(
            "tools.sleep.org.work.wait",
            json!({}),
            ExecuteOptions {
                auto_approve: true,
                timeout: Some(Duration::from_millis(40)),
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect_err("timeout");
    assert!(matches!(err, ExecutorError::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn cancel_in_flight_execute() {
    let started = Arc::new(Notify::new());
    let exec = wired(
        Arc::new(SleepPlugin {
            delay: Duration::from_secs(2),
            started: Arc::clone(&started),
        }),
        Limits::production(),
        "sleep",
    )
    .await;
    let cancel = CancellationToken::new();
    let exec_bg = exec.clone();
    let token = cancel.clone();
    let handle = tokio::spawn(async move {
        exec_bg
            .execute_with_cancel(
                "tools.sleep.org.work.wait",
                json!({}),
                ExecuteOptions {
                    auto_approve: true,
                    timeout: Some(Duration::from_secs(5)),
                    ..ExecuteOptions::default()
                },
                token,
            )
            .await
    });
    started.notified().await;
    cancel.cancel();
    let err = handle.await.expect("join").expect_err("cancelled");
    assert!(matches!(err, ExecutorError::Cancelled), "{err:?}");
}

#[tokio::test]
async fn idempotent_retry_reuses_execution() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let key = IdempotencyKey::new("k1").expect("key");
    let opts = ExecuteOptions {
        auto_approve: true,
        idempotency_key: Some(key),
        ..ExecuteOptions::default()
    };
    let a = exec
        .execute("tools.echo.org.work.ping", json!({"n": 7}), opts.clone())
        .await
        .expect("a");
    let b = exec
        .execute("tools.echo.org.work.ping", json!({"n": 99}), opts)
        .await
        .expect("b");
    let id_a = match a {
        Outcome::Completed { execution_id, .. } => execution_id,
        Outcome::Paused { execution } => execution.id,
    };
    let id_b = match b {
        Outcome::Completed { execution_id, .. } => execution_id,
        Outcome::Paused { execution } => execution.id,
    };
    assert_eq!(id_a.as_str(), id_b.as_str());
}

#[tokio::test]
async fn policy_block() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern":"*","action":"block","owner":"org"}),
        ExecuteOptions {
            auto_approve: true,
            ..ExecuteOptions::default()
        },
    )
    .await
    .expect("policy");
    let err = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 1}),
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect_err("blocked");
    assert!(matches!(err, ExecutorError::ToolBlocked { .. }), "{err:?}");
}

#[tokio::test]
async fn approval_pause_and_resume() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern":"echo.*","action":"require_approval","owner":"org"}),
        ExecuteOptions {
            auto_approve: true,
            ..ExecuteOptions::default()
        },
    )
    .await
    .expect("policy");
    let paused = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 3}),
            ExecuteOptions::default(),
        )
        .await
        .expect("pause");
    let Outcome::Paused { execution } = paused else {
        panic!("expected pause");
    };
    let done = exec
        .resume(&execution.id, ResumeAction::Accept)
        .await
        .expect("resume");
    match done {
        Outcome::Completed { result, .. } => match result {
            ToolResult::Ok { data, .. } => assert_eq!(data["n"], 3),
            ToolResult::Err { error } => panic!("{error:?}"),
        },
        Outcome::Paused { .. } => panic!("still paused"),
    }
}

#[tokio::test]
async fn resume_content_and_persist_session() {
    use executor_core::PersistChoice;

    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern":"echo.*","action":"require_approval","owner":"org"}),
        ExecuteOptions {
            auto_approve: true,
            ..ExecuteOptions::default()
        },
    )
    .await
    .expect("policy");
    let paused = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 3}),
            ExecuteOptions::default(),
        )
        .await
        .expect("pause");
    let Outcome::Paused { execution } = paused else {
        panic!("expected pause");
    };
    match &execution.reason {
        executor_core::PauseReason::Approval { schema, .. } => {
            assert!(schema.is_some(), "approval form schema");
        }
        other => panic!("expected approval form, got {other:?}"),
    }
    let done = exec
        .resume_request(
            &execution.id,
            ResumeRequest {
                action: ResumeAction::Accept,
                content: Some(json!({"n": 9, "persist": "session"})),
                persist: Some(PersistChoice::Session),
            },
        )
        .await
        .expect("resume");
    match done {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => assert_eq!(data["n"], 9, "{data}"),
        other => panic!("{other:?}"),
    }
    let second = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 1}),
            ExecuteOptions::default(),
        )
        .await
        .expect("session skip");
    assert!(
        matches!(
            second,
            Outcome::Completed {
                result: ToolResult::Ok { .. },
                ..
            }
        ),
        "{second:?}"
    );
}

#[tokio::test]
async fn persist_always_writes_approve_policy() {
    use executor_core::PersistChoice;

    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    exec.execute(
        "executor.coreTools.policies.create",
        json!({"pattern":"echo.*","action":"require_approval","owner":"org"}),
        ExecuteOptions {
            auto_approve: true,
            ..ExecuteOptions::default()
        },
    )
    .await
    .expect("policy");
    let paused = exec
        .execute(
            "tools.echo.org.work.ping",
            json!({"n": 3}),
            ExecuteOptions::default(),
        )
        .await
        .expect("pause");
    let Outcome::Paused { execution } = paused else {
        panic!("expected pause");
    };
    exec.resume_request(
        &execution.id,
        ResumeRequest {
            action: ResumeAction::Accept,
            content: None,
            persist: Some(PersistChoice::Always),
        },
    )
    .await
    .expect("resume");
    let policies = exec.list_policies().expect("policies");
    assert!(
        policies.iter().any(|p| p.action == PolicyAction::Approve),
        "{policies:?}"
    );
}

#[test]
fn policy_action_is_closed() {
    assert_eq!(PolicyAction::Block.restriction_rank(), 3);
}

#[tokio::test]
async fn code_mode_compiles_and_calls() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let src = executor_core::compile_call("tools.echo.org.work.ping", &json!({"n": 9}));
    let out = exec
        .run_code(
            &src,
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect("code");
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => {
            assert_eq!(data["n"], 9);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn code_mode_js_control_flow() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let src = r"
        const ping = await tools.echo.org.work.ping({ n: 4 });
        if (ping.n !== 4) throw new Error('bad ping');
        return { doubled: ping.n * 2 };
    ";
    let out = exec
        .run_code(
            src,
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect("quickjs");
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => assert_eq!(data["doubled"], 8, "{data}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn ranked_search_and_describe_tool() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let page = exec
        .search_ranked(&json!({"query": "ping", "limit": 5}))
        .expect("search");
    let items = page["items"].as_array().expect("items");
    assert!(!items.is_empty(), "{page}");
    assert_eq!(items[0]["name"], "ping");
    let shape = exec.describe_shape("echo.org.work.ping");
    assert!(
        shape["inputTypeScript"]
            .as_str()
            .is_some_and(|s| s.contains("n?")),
        "{shape}"
    );
}

#[tokio::test]
async fn typescript_strip_in_code_mode() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let src = "const n: number = 3;\nreturn n;";
    let out = exec
        .run_code(
            src,
            ExecuteOptions {
                auto_approve: true,
                ..ExecuteOptions::default()
            },
        )
        .await
        .expect("strip");
    match out {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => assert_eq!(data, json!(3)),
        other => panic!("{other:?}"),
    }
}

fn auto() -> ExecuteOptions {
    ExecuteOptions {
        auto_approve: true,
        ..ExecuteOptions::default()
    }
}

async fn data_ok(exec: &Executor, path: &str, args: serde_json::Value) -> serde_json::Value {
    match exec.execute(path, args, auto()).await.expect(path) {
        Outcome::Completed {
            result: ToolResult::Ok { data, .. },
            ..
        } => data,
        other => panic!("{path}: {other:?}"),
    }
}

#[tokio::test]
async fn detect_and_policy_update() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let hits = data_ok(
        &exec,
        "executor.coreTools.integrations.detect",
        json!({"url": "echo://box"}),
    )
    .await;
    assert_eq!(hits["results"][0]["slug"], "echo");
    let created = data_ok(
        &exec,
        "executor.coreTools.policies.create",
        json!({"pattern": "echo.*", "action": "approve", "owner": "org"}),
    )
    .await;
    let id = created["id"].as_str().expect("id");
    let updated = data_ok(
        &exec,
        "executor.coreTools.policies.update",
        json!({"id": id, "action": "block"}),
    )
    .await;
    assert_eq!(updated["action"], "block");
}

#[tokio::test]
async fn oauth_client_and_providers() {
    let exec = wired(Arc::new(EchoPlugin), Limits::production(), "echo").await;
    let client = data_ok(
        &exec,
        "executor.coreTools.oauth.clients.create",
        json!({
            "slug": "pub",
            "authorizationUrl": "https://example.test/auth",
            "tokenUrl": "https://example.test/token",
            "clientId": "cid"
        }),
    )
    .await;
    assert_eq!(client["clientId"], "cid");
    let providers = data_ok(&exec, "executor.coreTools.providers.list", json!({})).await;
    assert!(
        providers["providers"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "{providers}"
    );
}
