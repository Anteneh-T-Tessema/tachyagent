pub mod billing;
pub mod cost_model;
mod event;
mod license;
mod logger;
pub mod metering;
pub mod oauth;
mod policy;
mod policy_engine;
mod policy_file;
mod rbac;
mod security;
mod sentinel;
pub mod sso;
pub mod telemetry;

pub use billing::{
    BillingBackend, BillingError, BillingReport, BillingStatus, StripeBillingConnector,
    SubscriptionInfo,
};
pub use event::{verify_audit_chain, AsymmetricSigner, AuditEvent, AuditEventKind, AuditSeverity};
pub use license::{LicenseData, LicenseFile, LicenseStatus, LicenseTier};
pub use logger::{
    AuditLogger, AuditSink, FileAuditSink, HttpAuditSink, MemoryAuditSink, S3AuditSink,
};
pub use metering::{MeteringError, MeteringService, UsageAggregate, UsageEvent, UsageEventType};
pub use oauth::{OAuthClientConfig, OAuthManager, OAuthProvider, OAuthSession};
pub use policy::{GovernancePolicy, GovernanceViolation, ToolGovernanceRule};
pub use policy_engine::{
    FilePatch, PolicyAction, PolicyDecision, PolicyEngine, PolicyRule, PolicyRuleType,
};
pub use policy_file::PolicyFile;
pub use rbac::{
    check_permission, check_team_permission, default_quota_for_role, AccessResult, Action,
    QuotaResult, QuotaStore, Role, RoleQuota, User, UserStore, UserUsage,
};
pub use security::{
    hash_api_key, hash_text, is_safe_path, redact_sensitive, sanitize_prompt, verify_api_key,
    RateDecision, RateLimiter, RateTier, TieredRateLimiter,
};
pub use sentinel::{ComplianceSentinel, SecurityRule, SecurityViolation, ViolationAction};
pub use sso::{SamlAssertion, SsoConfig, SsoManager, SsoSession};
pub use telemetry::{TelemetryCollector, TelemetryConfig, TelemetryEvent, TelemetrySummary};
