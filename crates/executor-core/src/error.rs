//! Recoverable failures. Domain tool failures (`ok: false`) are [`crate::ToolResult`], not these.

use std::fmt::Display;

use thiserror::Error;

use crate::{IntegrationSlug, ToolAddress};

/// An identifier failed to parse.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{kind} {value:?} is not a valid identifier: {reason}")]
pub struct InvalidId {
    /// Which identifier.
    pub kind: &'static str,
    /// The rejected text.
    pub value: String,
    /// Why it was rejected.
    pub reason: &'static str,
}

impl InvalidId {
    pub(crate) fn new(kind: &'static str, value: impl Into<String>, reason: &'static str) -> Self {
        Self {
            kind,
            value: value.into(),
            reason,
        }
    }
}

/// Persistence failure. Not a domain error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("storage: {message}")]
pub struct StorageError {
    /// Human-readable cause.
    pub message: String,
}

impl StorageError {
    /// Build a storage error from any displayable cause.
    pub fn new(message: impl Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// Failures on the executor's error channel (infra, policy, missing catalog rows).
#[derive(Clone, Debug, Error)]
pub enum ExecutorError {
    /// No tool exists at this address (or the CLI path did not resolve uniquely).
    #[error("tool not found: {address}")]
    ToolNotFound {
        /// Address or path the caller asked for.
        address: String,
        /// Nearby catalog addresses, if any.
        suggestions: Vec<String>,
        /// Extra context.
        reason: Option<String>,
    },
    /// A `block` policy matched.
    #[error("tool blocked: {address} (pattern {pattern})")]
    ToolBlocked {
        /// Tool that was gated.
        address: String,
        /// Matching policy pattern.
        pattern: String,
        /// Policy id.
        policy_id: String,
    },
    /// Integration slug is unknown.
    #[error("integration not found: {0}")]
    IntegrationNotFound(IntegrationSlug),
    /// Connection identity is unknown.
    #[error("connection not found: {0}")]
    ConnectionNotFound(String),
    /// Plugin id is not loaded in this executor.
    #[error("plugin not loaded: {0}")]
    PluginNotLoaded(String),
    /// Secret backend is unknown.
    #[error("credential provider not registered: {0}")]
    ProviderNotRegistered(String),
    /// Secret resolution failed.
    #[error("credential resolution failed: {0}")]
    CredentialResolution(String),
    /// Arguments failed JSON Schema validation.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// Policy pattern is illegal.
    #[error("invalid policy pattern: {0}")]
    InvalidPattern(String),
    /// Catalog write rejected.
    #[error("catalog conflict: {0}")]
    Conflict(String),
    /// Built-in namespace cannot be removed.
    #[error("integration {0} cannot be removed")]
    RemovalNotAllowed(IntegrationSlug),
    /// Identifier parse error.
    #[error(transparent)]
    InvalidId(#[from] InvalidId),
    /// Storage.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Plugin invocation failed at the infrastructure layer.
    #[error("plugin error: {0}")]
    Plugin(String),
    /// Resume targeted an unknown or consumed execution.
    #[error("execution not found: {0}")]
    ExecutionNotFound(String),
    /// Resume targeted an execution that is not paused.
    #[error("execution {0} is not paused")]
    NotPaused(String),
    /// In-flight execute slots are exhausted. Callers must back off.
    #[error(
        "overloaded: {in_flight} in-flight (max {max_in_flight}); retry after {retry_after_ms}ms"
    )]
    Overloaded {
        /// Current in-flight executes.
        in_flight: u32,
        /// Configured cap.
        max_in_flight: u32,
        /// Hint for retry-after.
        retry_after_ms: u32,
    },
    /// Deadline expired.
    #[error("timeout after {timeout_ms}ms")]
    Timeout {
        /// Budget that elapsed.
        timeout_ms: u64,
    },
    /// Caller cancelled the invoke.
    #[error("cancelled")]
    Cancelled,
    /// Catalog or spec exceeded a configured bound.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

impl ExecutorError {
    /// Missing tool with optional suggestions.
    #[must_use]
    pub fn tool_not_found(address: impl Into<String>, suggestions: Vec<String>) -> Self {
        Self::ToolNotFound {
            address: address.into(),
            suggestions,
            reason: None,
        }
    }

    /// Missing tool with a reason.
    #[must_use]
    pub fn tool_not_found_reason(address: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ToolNotFound {
            address: address.into(),
            suggestions: Vec::new(),
            reason: Some(reason.into()),
        }
    }

    /// Format a [`ToolAddress`] missing from the catalog.
    #[must_use]
    pub fn missing_address(address: &ToolAddress) -> Self {
        Self::tool_not_found(address.to_string(), Vec::new())
    }
}
