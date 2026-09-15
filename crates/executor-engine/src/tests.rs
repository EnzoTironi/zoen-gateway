//! Failure, cancellation, timeout, and overload coverage — not only the happy path.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use executor_core::{
    AuthMethod, ConnectionInput, ConnectionName, Detection, ExecuteOptions, ExecutorError,
    HealthCheckCtx, IdempotencyKey, IntegrationConfig, IntegrationPlugin, IntegrationSlug,
    InvokeCtx, Limits, Outcome, PluginError, PluginId, PolicyAction, RegisterIntegration,
    ResolveToolsCtx, ResolvedTools, ResumeAction, ToolDef, ToolName, ToolResult,
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

    fn detect(&self, _: &str) -> Option<Detection> {
        None
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

#[test]
fn policy_action_is_closed() {
    assert_eq!(PolicyAction::Block.restriction_rank(), 3);
}
