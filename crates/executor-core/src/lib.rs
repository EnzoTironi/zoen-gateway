//! Domain types for Executor: catalog identity, policy, execution, and the plugin seam.
//!
//! This crate is IO-free. Persistence, HTTP, and process spawning live above it.

#![allow(clippy::result_large_err)] // ExecutorError carries address + suggestions; boxing would hide context.
#![allow(clippy::module_name_repetitions)] // Domain names follow Executor vocabulary (`ToolAddress`, `PolicyAction`).
#![allow(clippy::derive_partial_eq_without_eq)] // `serde_json::Value` is not `Eq`; opaque config uses it.

mod address;
mod auth;
mod connection;
mod ema;
mod error;
mod execution;
mod id;
mod integration;
mod json_schema;
mod json_ts;
mod limits;
mod metrics;
mod owner;
mod path;
mod plugin;
mod policy;
mod position;
mod scope;
mod search;
mod secret;
mod store;
mod tool;
mod toolkit;
mod www_authenticate;

pub use address::{
    ConnectionAddress, ParsedToolAddress, ToolAddress, connection_address, parse_tool_address,
    tool_address,
};
pub use auth::{AuthKind, AuthMethod, AuthPlacement, AuthTemplateSlug, Carrier, NO_AUTH_TEMPLATE};
pub use connection::{
    Connection, ConnectionInput, ConnectionRef, CredentialMap, HealthVerdict, IdentityLabel,
};
pub use ema::{
    DEFAULT_SUBJECT_TOKEN_TYPE, ENTERPRISE_MANAGED_PROVIDER_STATE_KEY, EmaError, EmaStep,
    ID_JAG_GRANT_PROFILE, ID_JAG_TOKEN_TYPE, ID_JAG_TOKEN_TYPE_SENTINEL, JWT_BEARER_GRANT_TYPE,
    TOKEN_EXCHANGE_GRANT_TYPE, supports_id_jag_grant_profile,
};
pub use error::{ExecutorError, InvalidId, StorageError};
pub use execution::{
    ExecuteOptions, ExecutionId, ExecutionState, IdempotencyKey, Outcome, PauseReason,
    PausedExecution, PersistChoice, ResumeAction, ResumeRequest, ToolError, ToolFile, ToolHttpMeta,
    ToolResult, unix_now_ms,
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
pub use json_ts::json_schema_to_typescript;
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
pub use scope::{
    InsufficientScope, OAUTH_SCOPE_INSUFFICIENT, detect_insufficient_scope, tool_error_from_http,
};
pub use search::{SearchArgs, SearchPage, SearchableTool, ToolDiscovery, search_tools};
pub use secret::{SecretRef, strip_secret_refs};
pub use store::{BlobStore, CatalogStore, MemoryCatalog};
pub use tool::{Tool, ToolAnnotations, ToolDef, ToolListFilter};
pub use toolkit::{
    KV_OAUTH_CLIENTS, KV_OAUTH_SESSIONS, KV_SESSION_APPROVALS, KV_SUBJECTS, KV_TOOLKITS,
    LOCAL_SUBJECT, Toolkit, ToolkitPolicy,
};
pub use www_authenticate::{AuthChallenge, parse_challenges};
