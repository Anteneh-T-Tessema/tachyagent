//! SaaS platform property tests.
//!
//! Feature: product-hardening-v3
//! Properties 20–23: SaaSPlatform correctness.
//! Validates: Requirements 8.1–8.5

use proptest::prelude::*;
use daemon::{SaaSPlatform, SaaSError};

const SECRET: &str = "test-jwt-secret-key";

fn make_platform() -> SaaSPlatform {
    SaaSPlatform::new(SECRET)
}

// ---------------------------------------------------------------------------
// Property 20: Tenant data isolation
// ---------------------------------------------------------------------------

#[test]
fn tenant_data_isolation_basic() {
    // Feature: product-hardening-v3, Property 20: Tenant data isolation
    let mut platform = make_platform();

    let (tenant_a, _) = platform.signup("alice@example.com", "hash-a").unwrap();
    let (tenant_b, _) = platform.signup("bob@example.com", "hash-b").unwrap();

    assert_ne!(tenant_a.id, tenant_b.id, "tenants must have distinct IDs");

    let dash_a = platform.dashboard(&tenant_a.id).unwrap();
    assert_eq!(dash_a.tenant_id, tenant_a.id);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 20: Each signup produces a distinct tenant ID regardless of email.
    ///
    /// Feature: product-hardening-v3, Property 20: Tenant data isolation
    #[test]
    fn prop_tenant_ids_are_unique(
        suffix_a in "[a-z]{3,8}",
        suffix_b in "[a-z]{3,8}",
    ) {
        prop_assume!(suffix_a != suffix_b);
        let mut platform = make_platform();

        let (ta, _) = platform.signup(&format!("{suffix_a}@example.com"), "hash").unwrap();
        let (tb, _) = platform.signup(&format!("{suffix_b}@example.com"), "hash").unwrap();

        prop_assert_ne!(ta.id, tb.id, "tenant IDs must be distinct");
    }
}

// ---------------------------------------------------------------------------
// Property 21: Tenant signup creates all required resources
// ---------------------------------------------------------------------------

#[test]
fn signup_creates_tenant_with_workspace() {
    // Feature: product-hardening-v3, Property 21: Tenant signup creates all required resources
    let mut platform = make_platform();
    let (tenant, token) = platform.signup("user@example.com", "password_hash").unwrap();

    assert!(!tenant.id.is_empty(), "tenant ID must be non-empty");
    assert!(!tenant.workspace_dir.as_os_str().is_empty(), "workspace dir must be set");
    assert!(!tenant.ollama_endpoint.is_empty(), "ollama endpoint must be set");
    assert!(!token.is_empty(), "JWT token must be returned");
    assert!(tenant.resource_limits.max_concurrent_agents > 0);
    assert!(tenant.resource_limits.max_tokens_per_day > 0);
}

#[test]
fn duplicate_email_rejected() {
    // Feature: product-hardening-v3, Property 21: Tenant signup creates all required resources
    let mut platform = make_platform();
    platform.signup("dup@example.com", "hash").unwrap();
    let result = platform.signup("dup@example.com", "hash2");
    assert!(
        matches!(result, Err(SaaSError::DuplicateEmail)),
        "duplicate email must be rejected"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 21: Every signup produces a non-empty workspace dir and ollama endpoint.
    ///
    /// Feature: product-hardening-v3, Property 21: Tenant signup creates all required resources
    #[test]
    fn prop_signup_sets_required_fields(suffix in "[a-z]{4,10}") {
        let mut platform = make_platform();
        let (tenant, token) = platform.signup(&format!("{suffix}@acme.com"), "ph").unwrap();

        prop_assert!(!tenant.id.is_empty());
        prop_assert!(!tenant.workspace_dir.as_os_str().is_empty());
        prop_assert!(!tenant.ollama_endpoint.is_empty());
        prop_assert!(!token.is_empty());
        prop_assert!(tenant.resource_limits.max_concurrent_agents > 0);
    }
}

// ---------------------------------------------------------------------------
// Property 22: JWT authentication round-trip
// ---------------------------------------------------------------------------

#[test]
fn jwt_round_trip_basic() {
    // Feature: product-hardening-v3, Property 22: JWT authentication round-trip
    let mut platform = SaaSPlatform::with_expiry(SECRET, 3600);
    let (tenant, token) = platform.signup("jwt@example.com", "hash").unwrap();

    let claims = platform.validate_jwt(&token).unwrap();
    assert_eq!(claims.tenant_id, tenant.id);
}

#[test]
fn invalid_jwt_rejected() {
    // Feature: product-hardening-v3, Property 22: JWT authentication round-trip
    let platform = SaaSPlatform::new(SECRET);
    assert!(matches!(platform.validate_jwt("not.a.jwt"), Err(SaaSError::InvalidJwt(_))));
}

#[test]
fn jwt_from_different_secret_rejected() {
    // Feature: product-hardening-v3, Property 22: JWT authentication round-trip
    let mut platform_a = SaaSPlatform::new("secret-a");
    let platform_b = SaaSPlatform::new("secret-b");
    let (_, token) = platform_a.signup("cross@example.com", "hash").unwrap();
    assert!(matches!(platform_b.validate_jwt(&token), Err(SaaSError::InvalidJwt(_))));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 22: For any valid signup, the JWT validates and maps to the same tenant.
    ///
    /// Feature: product-hardening-v3, Property 22: JWT authentication round-trip
    #[test]
    fn prop_jwt_round_trip(suffix in "[a-z]{3,8}") {
        let mut platform = SaaSPlatform::with_expiry(SECRET, 86_400);
        let (tenant, token) = platform.signup(&format!("{suffix}@prop.com"), "ph").unwrap();
        let claims = platform.validate_jwt(&token).unwrap();
        prop_assert_eq!(claims.tenant_id, tenant.id);
    }

    /// Property 22b: Arbitrary strings are always rejected as invalid JWTs.
    #[test]
    fn prop_garbage_jwt_rejected(garbage in "[a-zA-Z0-9+/]{5,50}") {
        // Feature: product-hardening-v3, Property 22: JWT authentication round-trip
        let platform = SaaSPlatform::new(SECRET);
        prop_assert!(matches!(platform.validate_jwt(&garbage), Err(SaaSError::InvalidJwt(_))));
    }
}

// ---------------------------------------------------------------------------
// Property 23: Resource limit enforcement
// ---------------------------------------------------------------------------

#[test]
fn resource_limit_tokens_enforced() {
    // Feature: product-hardening-v3, Property 23: Resource limit enforcement
    let mut platform = make_platform();
    let (tenant, _) = platform.signup("limited@example.com", "hash").unwrap();

    // Push tokens past the default limit (1M) by recording massive usage
    platform.record_usage(&tenant.id, 2_000_000, false).unwrap();

    let result = platform.check_limits(&tenant.id, "token_usage");
    assert!(
        matches!(result, Err(SaaSError::ResourceLimitExceeded(_))),
        "exceeding token limit must return ResourceLimitExceeded"
    );
}

#[test]
fn resource_limit_concurrent_agents_enforced() {
    // Feature: product-hardening-v3, Property 23: Resource limit enforcement
    let mut platform = make_platform();
    let (tenant, _) = platform.signup("agents@example.com", "hash").unwrap();

    // Record 10 agent runs (default max_concurrent_agents is 4)
    for _ in 0..10 {
        platform.record_usage(&tenant.id, 0, true).unwrap();
    }

    let result = platform.check_limits(&tenant.id, "agent_run");
    assert!(
        matches!(result, Err(SaaSError::ResourceLimitExceeded(_))),
        "exceeding concurrent agent limit must return ResourceLimitExceeded"
    );
}

#[test]
fn within_limits_returns_ok() {
    // Feature: product-hardening-v3, Property 23: Resource limit enforcement
    let mut platform = make_platform();
    let (tenant, _) = platform.signup("ok@example.com", "hash").unwrap();
    // Zero usage — should be within limits
    assert!(platform.check_limits(&tenant.id, "agent_run").is_ok());
}
