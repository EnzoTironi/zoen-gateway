//! Domain types for Executor: catalog identity, policy, execution, and the plugin seam.
//!
//! This crate is IO-free. Persistence, HTTP, and process spawning live above it.

#![allow(clippy::result_large_err)] // ExecutorError carries address + suggestions; boxing would hide context.
#![allow(clippy::module_name_repetitions)] // Domain names follow Executor vocabulary (`ToolAddress`, `PolicyAction`).
#![allow(clippy::derive_partial_eq_without_eq)] // `serde_json::Value` is not `Eq`; opaque config uses it.

mod address;
mod auth;
mod connection;
mod error;
mod execution;
mod id;
mod integration;
mod json_schema;
mod limits;
mod metrics;
mod owner;
mod path;
mod plugin;
mod policy;
mod position;
mod secret;
mod store;
mod tool;

pub use address::{
    ConnectionAddress, ParsedToolAddress, ToolAddress, connection_address, parse_tool_address,
    tool_address,
};
pub use auth::{AuthKind, AuthMethod, AuthPlacement, AuthTemplateSlug, Carrier, NO_AUTH_TEMPLATE};
pub use connection::{
    Connection, ConnectionInput, ConnectionRef, CredentialMap, HealthVerdict, IdentityLabel,
};
pub use error::{ExecutorError, InvalidId, StorageError};
pub use execution::{
    ExecuteOptions, ExecutionId, ExecutionState, IdempotencyKey, Outcome, PauseReason,
    PausedExecution, ResumeAction, ToolError, ToolFile, ToolHttpMeta, ToolResult, unix_now_ms,
};
pub use id::{
    ArtifactId, ConnectionName, ElicitationId, IntegrationSlug, OAuthClientSlug, PluginId,
    PolicyId, ProviderItemId, ProviderKey, Subject, Tenant, ToolName,
};
pub use integration::{
    Detection, DetectionConfidence, Integration, IntegrationConfig, IntegrationRecord,
    RegisterIntegration,
};
pub use json_schema::{SchemaError, validate_against};
pub use limits::Limits;
pub use metrics::{AtomicMetrics, Metrics, MetricsSnapshot, NoopMetrics, names as metric_names};
pub use owner::Owner;
pub use path::{
    Invocation, ToolPathChild, ToolPathError, ToolPathInspection, build_tool_path, compile_call,
    inspect_tool_path, resolve_invocation,
};
pub use plugin::{
    CredentialMapValues, HealthCheckCtx, IntegrationPlugin, InvokeCtx, PluginError,
    ResolveToolsCtx, ResolvedTools,
};
pub use policy::{
    EffectivePolicy, PolicyAction, PolicyMatch, PolicyPattern, PolicySource, ToolPolicy,
    compare_policy_row, effective_policy, effective_policy_from_sorted, is_valid_pattern,
    match_pattern, pattern_specificity, position_for_new_pattern, resolve_tool_policy,
};
pub use position::generate_key_between;
pub use secret::{SecretRef, strip_secret_refs};
pub use store::{BlobStore, CatalogStore, MemoryCatalog};
pub use tool::{Tool, ToolAnnotations, ToolDef, ToolListFilter};
