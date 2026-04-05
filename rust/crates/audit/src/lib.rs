mod event;
mod license;
mod logger;
mod policy;
mod policy_engine;
mod policy_file;
mod rbac;
mod security;
pub mod sso;
pub mod telemetry;

pub use event::{AuditEvent, AuditEventKind, AuditSeverity, verify_audit_chain};
pub use license::{LicenseData, LicenseFile, LicenseStatus, LicenseTier};
pub use logger::{AuditLogger, FileAuditSink, AuditSink};
pub use policy::{GovernancePolicy, GovernanceViolation, ToolGovernanceRule};
pub use policy_engine::{PolicyEngine, PolicyDecision, PolicyRule, PolicyRuleType, PolicyAction, FilePatch};
pub use policy_file::PolicyFile;
pub use rbac::{check_permission, Action, AccessResult, Role, User, UserStore};
pub use security::{
    hash_api_key, verify_api_key, is_safe_path, redact_sensitive, sanitize_prompt,
    RateLimiter,
};
pub use sso::{SsoConfig, SsoManager, SsoSession, SamlAssertion};
pub use telemetry::{TelemetryCollector, TelemetryConfig, TelemetryEvent, TelemetrySummary};
