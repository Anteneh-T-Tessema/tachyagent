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
pub use event::{AuditEvent, AuditEventKind, AuditSeverity, verify_audit_chain, AsymmetricSigner};
pub use license::{LicenseData, LicenseFile, LicenseStatus, LicenseTier};
pub use logger::{AuditLogger, AuditSink, FileAuditSink, HttpAuditSink, MemoryAuditSink, S3AuditSink};
pub use metering::{MeteringError, MeteringService, UsageAggregate, UsageEvent, UsageEventType};
pub use policy::{GovernancePolicy, GovernanceViolation, ToolGovernanceRule};
pub use policy_engine::{PolicyEngine, PolicyDecision, PolicyRule, PolicyRuleType, PolicyAction, FilePatch};
pub use policy_file::PolicyFile;
pub use rbac::{
    check_permission, check_team_permission, default_quota_for_role,
    Action, AccessResult, QuotaResult, QuotaStore, Role, RoleQuota, User, UserStore, UserUsage,
};
pub use security::{
    hash_api_key, hash_text, verify_api_key, is_safe_path, redact_sensitive, sanitize_prompt,
    RateDecision, RateLimiter, RateTier, TieredRateLimiter,
};
pub use oauth::{OAuthClientConfig, OAuthManager, OAuthProvider, OAuthSession};
pub use sso::{SsoConfig, SsoManager, SsoSession, SamlAssertion};
pub use sentinel::{ComplianceSentinel, ViolationAction, SecurityRule, SecurityViolation};
pub use telemetry::{TelemetryCollector, TelemetryConfig, TelemetryEvent, TelemetrySummary};
