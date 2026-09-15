//! Execution state machine and tool results.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static EXEC_SEQ: AtomicU64 = AtomicU64::new(1);

fn mint_id(prefix: &str) -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = EXEC_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{ns:x}_{seq:x}")
}

/// Correlation id for one `execute` (`exec_<uuid>`).
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExecutionId(String);

impl ExecutionId {
    /// Mint a new id.
    #[must_use]
    pub fn mint() -> Self {
        Self(mint_id("exec"))
    }

    /// Parse a previously minted id.
    ///
    /// # Errors
    ///
    /// Empty.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, crate::InvalidId> {
        let value = raw.as_ref().trim();
        if value.is_empty() {
            return Err(crate::InvalidId::new(
                "execution id",
                raw.as_ref(),
                "must be non-empty",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Borrow the wire form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// HTTP transport facts beside a successful payload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolHttpMeta {
    /// Status code.
    pub status: u16,
    /// Response headers (lowercased names).
    pub headers: Vec<(String, String)>,
}

/// Expected tool failure (rides the success channel as `ok: false`).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolError {
    /// Stable code (`http_error`, `graphql_error`, …).
    pub code: String,
    /// Human message.
    pub message: String,
    /// Upstream HTTP status when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Extra details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Hint that a retry may help.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

/// File payload (base64).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolFile {
    /// File name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// MIME type.
    pub mime_type: String,
    /// Always `base64`.
    pub encoding: String,
    /// Base64 bytes.
    pub data: String,
    /// Raw size before encoding.
    pub byte_length: u64,
}

/// Domain success or expected failure. Infra defects use [`crate::ExecutorError`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolResult {
    /// Handler succeeded.
    Ok {
        /// Payload.
        data: Value,
        /// Optional HTTP meta.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http: Option<ToolHttpMeta>,
    },
    /// Expected failure.
    Err {
        /// Error body.
        error: ToolError,
    },
}

impl ToolResult {
    /// Successful payload.
    #[must_use]
    pub const fn ok(data: Value) -> Self {
        Self::Ok { data, http: None }
    }

    /// Successful payload with HTTP meta.
    #[must_use]
    pub const fn ok_http(data: Value, http: ToolHttpMeta) -> Self {
        Self::Ok {
            data,
            http: Some(http),
        }
    }

    /// Expected failure.
    #[must_use]
    pub const fn fail(error: ToolError) -> Self {
        Self::Err { error }
    }

    /// Wire envelope matching the original `{ ok, data | error }`.
    #[must_use]
    pub fn envelope(&self) -> Value {
        match self {
            Self::Ok { data, http } => {
                let mut map = serde_json::Map::new();
                map.insert("ok".into(), Value::Bool(true));
                map.insert("data".into(), data.clone());
                if let Some(http) = http {
                    map.insert(
                        "http".into(),
                        serde_json::to_value(http).unwrap_or(Value::Null),
                    );
                }
                Value::Object(map)
            }
            Self::Err { error } => serde_json::json!({
                "ok": false,
                "error": error,
            }),
        }
    }
}

/// Why an execution paused.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    /// Policy `require_approval`.
    Approval {
        /// Address shown to the operator.
        address: String,
        /// Argument preview.
        args: Value,
        /// Optional description.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// OAuth / credential elicitation.
    Auth {
        /// Human message.
        message: String,
        /// URL to open, when applicable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    /// Structured form elicitation.
    Elicitation {
        /// Message.
        message: String,
        /// JSON Schema the operator should fill.
        schema: Value,
    },
}

/// A paused execution the caller must resume.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PausedExecution {
    /// Execution id.
    pub id: ExecutionId,
    /// Why.
    pub reason: PauseReason,
    /// Unix-ms expiry.
    pub expires_at_ms: u64,
}

/// Resume decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeAction {
    /// Proceed.
    Accept,
    /// Refuse.
    Decline,
    /// Cancel without running.
    Cancel,
}

/// Terminal or paused outcome of `execute`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Outcome {
    /// Finished.
    Completed {
        /// Result.
        result: ToolResult,
        /// Execution id.
        execution_id: ExecutionId,
    },
    /// Waiting on a human.
    Paused {
        /// Pause record.
        execution: PausedExecution,
    },
}

/// Durable execution record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ExecutionState {
    /// Currently running (not persisted mid-flight).
    Running,
    /// Done.
    Completed(ToolResult),
    /// Paused; resume consumes this.
    Paused {
        /// Pause.
        execution: PausedExecution,
        /// Address to invoke on accept.
        address: String,
        /// Args to invoke on accept.
        args: Value,
        /// Auto-approve after accept (already gated).
        approved: bool,
    },
}

impl Outcome {
    /// JSON object the CLI prints.
    #[must_use]
    pub fn cli_json(&self) -> Value {
        match self {
            Self::Completed {
                result,
                execution_id,
            } => serde_json::json!({
                "status": "completed",
                "executionId": execution_id.as_str(),
                "result": result.envelope(),
            }),
            Self::Paused { execution } => serde_json::json!({
                "status": "paused",
                "executionId": execution.id.as_str(),
                "interaction": execution.reason,
            }),
        }
    }
}

/// Caller-supplied key so retried executes reuse the first outcome.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Parse a non-empty key (max 256 bytes).
    ///
    /// # Errors
    ///
    /// Empty or too long.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, crate::InvalidId> {
        let value = raw.as_ref().trim();
        if value.is_empty() {
            return Err(crate::InvalidId::new(
                "idempotency key",
                raw.as_ref(),
                "must be non-empty",
            ));
        }
        if value.len() > 256 {
            return Err(crate::InvalidId::new(
                "idempotency key",
                value,
                "must be at most 256 bytes",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Borrow the key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Options for one `execute`. Absence means engine defaults from [`crate::Limits`].
#[derive(Clone, Debug, Default)]
pub struct ExecuteOptions {
    /// Skip the approval pause (operator CLI `--yes`, tests).
    pub auto_approve: bool,
    /// Override execute deadline.
    pub timeout: Option<std::time::Duration>,
    /// Retry key.
    pub idempotency_key: Option<IdempotencyKey>,
}

/// Unix time in milliseconds.
#[must_use]
pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
