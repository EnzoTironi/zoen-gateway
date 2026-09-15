//! Bounded `execute` / `resume`: semaphore, timeout, cancel, idempotency, policy.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use executor_core::{
    ConnectionRef, ExecuteOptions, ExecutionId, ExecutionState, ExecutorError, InvokeCtx, Outcome,
    PauseReason, PausedExecution, PolicyAction, ResumeAction, Tool, ToolError, ToolResult,
    metric_names, strip_secret_refs, unix_now_ms, validate_against,
};
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::lookup::Resolved;
use crate::{Inner, on_store};

struct InflightGuard {
    metrics: Arc<dyn executor_core::Metrics>,
    count: Arc<std::sync::atomic::AtomicU64>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let prev = self.count.fetch_sub(1, Ordering::Relaxed);
        let now = prev.saturating_sub(1);
        self.metrics
            .gauge(metric_names::IN_FLIGHT, i64::try_from(now).unwrap_or(0));
    }
}

impl Inner {
    #[instrument(skip(self, args, opts, cancel), fields(path))]
    pub(crate) async fn execute(
        &self,
        path: &str,
        args: Value,
        opts: ExecuteOptions,
        cancel: CancellationToken,
    ) -> Result<Outcome, ExecutorError> {
        let started = Instant::now();
        let timeout = opts.timeout.unwrap_or(self.limits.execute_timeout);
        let permit = self.acquire_permit().await?;
        let _inflight = self.arm_inflight();
        let run = self.execute_gated(path, args, &opts, timeout);
        let result = select_deadline(run, cancel, timeout).await;
        drop(permit);
        self.record_result(&result, started.elapsed());
        result
    }

    pub(crate) async fn resume(
        &self,
        id: &ExecutionId,
        action: ResumeAction,
        cancel: CancellationToken,
    ) -> Result<Outcome, ExecutorError> {
        let permit = self.acquire_permit().await?;
        let _inflight = self.arm_inflight();
        let timeout = self.limits.execute_timeout;
        let result = select_deadline(self.resume_gated(id.clone(), action), cancel, timeout).await;
        drop(permit);
        result
    }

    async fn acquire_permit(&self) -> Result<OwnedSemaphorePermit, ExecutorError> {
        let available = u32::try_from(self.semaphore.available_permits()).unwrap_or(u32::MAX);
        let in_flight = self.limits.max_in_flight.saturating_sub(available);
        if self.limits.acquire_timeout.is_zero() {
            return self.semaphore.clone().try_acquire_owned().map_err(|_| {
                self.metrics.counter(metric_names::EXECUTE_OVERLOAD, 1);
                ExecutorError::Overloaded {
                    in_flight,
                    max_in_flight: self.limits.max_in_flight,
                    retry_after_ms: 50,
                }
            });
        }
        match tokio::time::timeout(
            self.limits.acquire_timeout,
            self.semaphore.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(ExecutorError::Cancelled),
            Err(_) => {
                self.metrics.counter(metric_names::EXECUTE_OVERLOAD, 1);
                Err(ExecutorError::Overloaded {
                    in_flight: self.limits.max_in_flight,
                    max_in_flight: self.limits.max_in_flight,
                    retry_after_ms: 50,
                })
            }
        }
    }

    fn arm_inflight(&self) -> InflightGuard {
        let now = self.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        self.metrics
            .gauge(metric_names::IN_FLIGHT, i64::try_from(now).unwrap_or(0));
        InflightGuard {
            metrics: Arc::clone(&self.metrics),
            count: Arc::clone(&self.in_flight),
        }
    }

    fn record_result(&self, result: &Result<Outcome, ExecutorError>, elapsed: Duration) {
        let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.metrics.observe_ms("executor.execute.ms", ms);
        match result {
            Ok(_) => self.metrics.counter(metric_names::EXECUTE_OK, 1),
            Err(ExecutorError::Overloaded { .. }) => {}
            Err(ExecutorError::Timeout { .. }) => {
                self.metrics.counter(metric_names::EXECUTE_TIMEOUT, 1);
            }
            Err(ExecutorError::Cancelled) => {
                self.metrics.counter(metric_names::EXECUTE_CANCEL, 1);
            }
            Err(_) => self.metrics.counter(metric_names::EXECUTE_ERR, 1),
        }
    }

    async fn execute_gated(
        &self,
        path: &str,
        args: Value,
        opts: &ExecuteOptions,
        _timeout: Duration,
    ) -> Result<Outcome, ExecutorError> {
        self.reject_oversize(&args)?;
        if let Some(key) = &opts.idempotency_key
            && let Some(outcome) = self.replay_idempotent(key).await?
        {
            return Ok(outcome);
        }
        let resolved = self.resolve_path(path)?;
        let tool = resolved.tool().clone();
        Self::validate_args(&tool, &args)?;
        let id = ExecutionId::mint();
        if let Some(key) = &opts.idempotency_key
            && let Some(outcome) = self.claim_idempotent(key, &id).await?
        {
            return Ok(outcome);
        }
        let _ = self.gate(&tool)?;
        let outcome = self
            .dispatch(&resolved, args, &id, opts.auto_approve)
            .await?;
        self.persist_outcome(&id, &outcome).await?;
        Ok(outcome)
    }

    async fn resume_gated(
        &self,
        id: ExecutionId,
        action: ResumeAction,
    ) -> Result<Outcome, ExecutorError> {
        let store = Arc::clone(&self.store);
        let taken_id = id.clone();
        let state = on_store(store, move |s| s.take_execution(&taken_id)).await?;
        let Some(state) = state else {
            return Err(ExecutorError::ExecutionNotFound(id.to_string()));
        };
        match (state, action) {
            (ExecutionState::Paused { address, args, .. }, ResumeAction::Accept) => {
                let resolved = self.resolve_path(&address)?;
                self.dispatch(&resolved, args, &id, true).await
            }
            (ExecutionState::Paused { .. }, ResumeAction::Decline | ResumeAction::Cancel) => {
                Ok(Outcome::Completed {
                    result: ToolResult::fail(ToolError {
                        code: "declined".into(),
                        message: "execution was not approved".into(),
                        status: None,
                        details: None,
                        retryable: Some(false),
                    }),
                    execution_id: id,
                })
            }
            (_, _) => Err(ExecutorError::NotPaused(id.to_string())),
        }
    }

    fn reject_oversize(&self, args: &Value) -> Result<(), ExecutorError> {
        let bytes = serde_json::to_vec(args).unwrap_or_default().len();
        if bytes > self.limits.max_arg_bytes {
            return Err(ExecutorError::LimitExceeded(format!(
                "arguments are {bytes} bytes (max {})",
                self.limits.max_arg_bytes
            )));
        }
        Ok(())
    }

    fn validate_args(tool: &Tool, args: &Value) -> Result<(), ExecutorError> {
        if let Some(schema) = &tool.input_schema {
            validate_against(schema, args).map_err(|e| ExecutorError::InvalidArgs(e.0))?;
        } else if !args.is_null() && !args.is_object() {
            return Err(ExecutorError::InvalidArgs(
                "arguments must be a JSON object".into(),
            ));
        }
        Ok(())
    }

    fn gate(&self, tool: &Tool) -> Result<PolicyAction, ExecutorError> {
        let policies = self.store.list_policies()?;
        let effective = executor_core::effective_policy(
            &tool.cli_path(),
            &policies,
            executor_core::Owner::outer_rank,
            tool.annotations
                .as_ref()
                .and_then(|a| a.requires_approval)
                .unwrap_or(false),
        );
        if effective.action == PolicyAction::Block {
            return Err(ExecutorError::ToolBlocked {
                address: tool.cli_path(),
                pattern: effective.pattern.unwrap_or_default(),
                policy_id: effective.policy_id.unwrap_or_default(),
            });
        }
        Ok(effective.action)
    }

    async fn dispatch(
        &self,
        resolved: &Resolved,
        args: Value,
        id: &ExecutionId,
        auto_approve: bool,
    ) -> Result<Outcome, ExecutorError> {
        let tool = resolved.tool();
        let action = self.gate(tool)?;
        if action == PolicyAction::RequireApproval && !auto_approve {
            return self.pause_for_approval(tool, args, id);
        }
        let args = strip_secret_refs(args);
        match resolved {
            Resolved::Static(_) => {
                let result = self.invoke_static(&tool.cli_path(), &args).await?;
                Ok(Outcome::Completed {
                    result,
                    execution_id: id.clone(),
                })
            }
            Resolved::Dynamic(tool) => {
                let result = self.invoke_dynamic(tool, &args).await?;
                Ok(Outcome::Completed {
                    result,
                    execution_id: id.clone(),
                })
            }
        }
    }

    fn pause_for_approval(
        &self,
        tool: &Tool,
        args: Value,
        id: &ExecutionId,
    ) -> Result<Outcome, ExecutorError> {
        let ttl = u64::try_from(self.limits.approval_ttl.as_millis()).unwrap_or(u64::MAX);
        let execution = PausedExecution {
            id: id.clone(),
            reason: PauseReason::Approval {
                address: tool.cli_path(),
                args: args.clone(),
                description: tool
                    .annotations
                    .as_ref()
                    .and_then(|a| a.approval_description.clone()),
            },
            expires_at_ms: unix_now_ms().saturating_add(ttl),
        };
        let state = ExecutionState::Paused {
            execution: execution.clone(),
            address: resolved_invoke_path(tool),
            args,
            approved: false,
        };
        self.store.put_execution(id, state)?;
        Ok(Outcome::Paused { execution })
    }

    async fn invoke_dynamic(&self, tool: &Tool, args: &Value) -> Result<ToolResult, ExecutorError> {
        let id = ConnectionRef {
            owner: tool.owner,
            name: tool.connection.clone(),
            integration: tool.integration.clone(),
        };
        let store = Arc::clone(&self.store);
        let conn_id = id.clone();
        let slug = tool.integration.clone();
        let conn = on_store(Arc::clone(&store), move |s| s.get_connection(&conn_id))
            .await?
            .ok_or_else(|| ExecutorError::ConnectionNotFound(id.as_key()))?;
        let record = on_store(store, move |s| s.get_integration(&slug))
            .await?
            .ok_or_else(|| ExecutorError::IntegrationNotFound(tool.integration.clone()))?;
        let plugin = self
            .plugins
            .get(record.integration.kind.as_str())
            .ok_or_else(|| ExecutorError::PluginNotLoaded(record.integration.kind.to_string()))?;
        let values = self.resolve_secrets(&conn)?;
        let ctx = InvokeCtx {
            integration: &record,
            connection: &id,
            template: &conn.template,
            tool,
            args,
            values: &values,
            timeout: self.limits.http_timeout,
        };
        plugin
            .invoke(ctx)
            .await
            .map_err(|e| ExecutorError::Plugin(e.0))
    }

    async fn replay_idempotent(
        &self,
        key: &executor_core::IdempotencyKey,
    ) -> Result<Option<Outcome>, ExecutorError> {
        let store = Arc::clone(&self.store);
        let key_s = key.as_str().to_owned();
        let Some(id) = on_store(store, move |s| s.get_idempotency(&key_s)).await? else {
            return Ok(None);
        };
        self.outcome_from_execution(&id).await
    }

    async fn claim_idempotent(
        &self,
        key: &executor_core::IdempotencyKey,
        id: &ExecutionId,
    ) -> Result<Option<Outcome>, ExecutorError> {
        let store = Arc::clone(&self.store);
        let key_s = key.as_str().to_owned();
        let minted = id.clone();
        on_store(Arc::clone(&store), move |s| {
            s.put_idempotency(&key_s, &minted)
        })
        .await?;
        let store2 = Arc::clone(&self.store);
        let key_s = key.as_str().to_owned();
        let winner = on_store(store2, move |s| s.get_idempotency(&key_s))
            .await?
            .unwrap_or_else(|| id.clone());
        if winner.as_str() != id.as_str() {
            return self.outcome_from_execution(&winner).await;
        }
        let store3 = Arc::clone(&self.store);
        let running_id = id.clone();
        on_store(store3, move |s| {
            s.put_execution(&running_id, ExecutionState::Running)
        })
        .await?;
        Ok(None)
    }

    async fn outcome_from_execution(
        &self,
        id: &ExecutionId,
    ) -> Result<Option<Outcome>, ExecutorError> {
        for _ in 0..50 {
            let store = Arc::clone(&self.store);
            let lookup = id.clone();
            match on_store(store, move |s| s.get_execution(&lookup)).await? {
                Some(ExecutionState::Completed(result)) => {
                    return Ok(Some(Outcome::Completed {
                        result,
                        execution_id: id.clone(),
                    }));
                }
                Some(ExecutionState::Paused { execution, .. }) => {
                    return Ok(Some(Outcome::Paused { execution }));
                }
                Some(ExecutionState::Running) | None => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        Ok(None)
    }

    async fn persist_outcome(
        &self,
        id: &ExecutionId,
        outcome: &Outcome,
    ) -> Result<(), ExecutorError> {
        let state = match outcome {
            Outcome::Completed { result, .. } => ExecutionState::Completed(result.clone()),
            Outcome::Paused { execution } => ExecutionState::Paused {
                execution: execution.clone(),
                address: match &execution.reason {
                    PauseReason::Approval { address, .. } => address.clone(),
                    PauseReason::Auth { .. } | PauseReason::Elicitation { .. } => String::new(),
                },
                args: match &execution.reason {
                    PauseReason::Approval { args, .. } => args.clone(),
                    PauseReason::Auth { .. } | PauseReason::Elicitation { .. } => Value::Null,
                },
                approved: false,
            },
        };
        let store = Arc::clone(&self.store);
        let persist_id = id.clone();
        on_store(store, move |s| s.put_execution(&persist_id, state)).await
    }
}

fn resolved_invoke_path(tool: &Tool) -> String {
    if tool.static_tool {
        tool.cli_path()
    } else {
        tool.address.to_string()
    }
}

async fn select_deadline<F>(
    run: F,
    cancel: CancellationToken,
    timeout: Duration,
) -> Result<Outcome, ExecutorError>
where
    F: std::future::Future<Output = Result<Outcome, ExecutorError>>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(ExecutorError::Cancelled),
        () = tokio::time::sleep(timeout) => Err(ExecutorError::Timeout {
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        }),
        result = run => result,
    }
}
