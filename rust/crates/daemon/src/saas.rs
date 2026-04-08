//! `SaaS` Multi-Tenant Platform for the Tachy platform.
//! Tenant isolation, authentication, resource limits, and managed infrastructure.

use audit::hash_api_key;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A tenant in the `SaaS` platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub workspace_dir: PathBuf,
    pub ollama_endpoint: String,
    pub created_at: u64,
    pub resource_limits: ResourceLimits,
}

/// Resource limits for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_concurrent_agents: usize,
    pub max_tokens_per_day: u64,
    pub max_storage_bytes: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 4,
            max_tokens_per_day: 1_000_000,
            max_storage_bytes: 10_737_418_240, // 10 GB
        }
    }
}

/// Claims embedded in a JWT token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantClaims {
    pub tenant_id: String,
    pub email: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

/// Dashboard summary for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub tenant_id: String,
    pub agent_runs: u64,
    pub token_consumption: u64,
    pub active_members: u64,
    pub billing_status: String,
}

/// Errors from `SaaS` platform operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaaSError {
    TenantNotFound,
    ResourceLimitExceeded(String),
    InvalidJwt(String),
    DuplicateEmail,
    AuthenticationFailed,
}

impl std::fmt::Display for SaaSError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TenantNotFound => write!(f, "tenant not found"),
            Self::ResourceLimitExceeded(reason) => {
                write!(f, "resource limit exceeded: {reason}")
            }
            Self::InvalidJwt(reason) => write!(f, "invalid JWT: {reason}"),
            Self::DuplicateEmail => write!(f, "email already registered"),
            Self::AuthenticationFailed => write!(f, "authentication failed"),
        }
    }
}

/// Default JWT expiry: 24 hours in seconds.
const DEFAULT_JWT_EXPIRY_SECS: u64 = 86_400;

/// Internal record mapping email to tenant + password hash.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct UserRecord {
    tenant_id: String,
    email: String,
    password_hash: String,
}

/// The `SaaS` multi-tenant platform.
#[derive(Debug, Clone)]
pub struct SaaSPlatform {
    tenants: BTreeMap<String, Tenant>,
    users: BTreeMap<String, UserRecord>, // keyed by email
    jwt_secret: String,
    jwt_expiry_secs: u64,
    /// Per-tenant usage tracking for dashboard/limits.
    usage: BTreeMap<String, TenantUsage>,
}

/// Tracks current usage for a tenant (for limit checks and dashboard).
#[derive(Debug, Clone, Default)]
struct TenantUsage {
    concurrent_agents: usize,
    tokens_today: u64,
    storage_bytes: u64,
    agent_runs: u64,
    active_members: u64,
}

impl SaaSPlatform {
    /// Create a new `SaaS` platform with the given JWT secret.
    #[must_use] pub fn new(jwt_secret: &str) -> Self {
        Self {
            tenants: BTreeMap::new(),
            users: BTreeMap::new(),
            jwt_secret: jwt_secret.to_string(),
            jwt_expiry_secs: DEFAULT_JWT_EXPIRY_SECS,
            usage: BTreeMap::new(),
        }
    }

    /// Create a new `SaaS` platform with a custom JWT expiry.
    #[must_use] pub fn with_expiry(jwt_secret: &str, jwt_expiry_secs: u64) -> Self {
        Self {
            tenants: BTreeMap::new(),
            users: BTreeMap::new(),
            jwt_secret: jwt_secret.to_string(),
            jwt_expiry_secs,
            usage: BTreeMap::new(),
        }
    }

    /// Sign up a new tenant. Returns the tenant and a JWT token.
    ///
    /// - Generates tenant ID from email hash.
    /// - Creates a dedicated workspace directory path.
    /// - Sets default resource limits and managed Ollama endpoint.
    pub fn signup(
        &mut self,
        email: &str,
        password_hash: &str,
    ) -> Result<(Tenant, String), SaaSError> {
        if self.users.contains_key(email) {
            return Err(SaaSError::DuplicateEmail);
        }

        // Generate tenant ID from email hash (first 12 hex chars).
        let email_hash = hash_api_key(email);
        let tenant_id = format!("tenant-{}", &email_hash[..12]);

        let now = current_timestamp();

        let tenant = Tenant {
            id: tenant_id.clone(),
            name: email.to_string(),
            workspace_dir: PathBuf::from(format!("/data/tenants/{tenant_id}")),
            ollama_endpoint: "http://ollama-pool:11434".to_string(),
            created_at: now,
            resource_limits: ResourceLimits::default(),
        };

        self.tenants.insert(tenant_id.clone(), tenant.clone());
        self.users.insert(
            email.to_string(),
            UserRecord {
                tenant_id: tenant_id.clone(),
                email: email.to_string(),
                password_hash: password_hash.to_string(),
            },
        );
        self.usage.insert(
            tenant_id.clone(),
            TenantUsage {
                active_members: 1,
                ..Default::default()
            },
        );

        let token = self.generate_jwt(&tenant_id, email, now)?;
        Ok((tenant, token))
    }

    /// Authenticate a user by email and password hash. Returns a JWT.
    pub fn authenticate(
        &self,
        email: &str,
        password_hash: &str,
    ) -> Result<String, SaaSError> {
        let user = self
            .users
            .get(email)
            .ok_or(SaaSError::AuthenticationFailed)?;

        if user.password_hash != password_hash {
            return Err(SaaSError::AuthenticationFailed);
        }

        let now = current_timestamp();
        self.generate_jwt(&user.tenant_id, email, now)
    }

    /// Validate a JWT token and return the claims.
    pub fn validate_jwt(&self, token: &str) -> Result<TenantClaims, SaaSError> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(SaaSError::InvalidJwt("malformed token".to_string()));
        }

        let header_payload = format!("{}.{}", parts[0], parts[1]);
        let expected_sig = hmac_sha256(&self.jwt_secret, &header_payload);

        if parts[2] != expected_sig {
            return Err(SaaSError::InvalidJwt("invalid signature".to_string()));
        }

        // Decode payload (parts[1] is base64-encoded JSON).
        let payload_json = base64_decode(parts[1])
            .map_err(|e| SaaSError::InvalidJwt(format!("invalid payload: {e}")))?;

        let claims: TenantClaims = serde_json::from_str(&payload_json)
            .map_err(|e| SaaSError::InvalidJwt(format!("invalid claims: {e}")))?;

        let now = current_timestamp();
        if now > claims.expires_at {
            return Err(SaaSError::InvalidJwt("token expired".to_string()));
        }

        // Verify tenant still exists.
        if !self.tenants.contains_key(&claims.tenant_id) {
            return Err(SaaSError::TenantNotFound);
        }

        Ok(claims)
    }

    /// Check if a tenant's current usage is within resource limits.
    pub fn check_limits(
        &self,
        tenant_id: &str,
        action: &str,
    ) -> Result<(), SaaSError> {
        let tenant = self
            .tenants
            .get(tenant_id)
            .ok_or(SaaSError::TenantNotFound)?;

        let usage = self.usage.get(tenant_id).cloned().unwrap_or_default();
        let limits = &tenant.resource_limits;

        if action == "agent_run" && usage.concurrent_agents >= limits.max_concurrent_agents {
            return Err(SaaSError::ResourceLimitExceeded(
                "max concurrent agents exceeded".to_string(),
            ));
        }

        if usage.tokens_today >= limits.max_tokens_per_day {
            return Err(SaaSError::ResourceLimitExceeded(
                "max tokens per day exceeded".to_string(),
            ));
        }

        if usage.storage_bytes >= limits.max_storage_bytes {
            return Err(SaaSError::ResourceLimitExceeded(
                "max storage exceeded".to_string(),
            ));
        }

        Ok(())
    }

    /// Return a dashboard summary for a tenant.
    pub fn dashboard(
        &self,
        tenant_id: &str,
    ) -> Result<DashboardSummary, SaaSError> {
        if !self.tenants.contains_key(tenant_id) {
            return Err(SaaSError::TenantNotFound);
        }

        let usage = self.usage.get(tenant_id).cloned().unwrap_or_default();

        Ok(DashboardSummary {
            tenant_id: tenant_id.to_string(),
            agent_runs: usage.agent_runs,
            token_consumption: usage.tokens_today,
            active_members: usage.active_members,
            billing_status: "active".to_string(),
        })
    }

    /// Record usage for a tenant (for limit enforcement and dashboard).
    pub fn record_usage(
        &mut self,
        tenant_id: &str,
        tokens: u64,
        agent_run: bool,
    ) -> Result<(), SaaSError> {
        if !self.tenants.contains_key(tenant_id) {
            return Err(SaaSError::TenantNotFound);
        }
        let usage = self.usage.entry(tenant_id.to_string()).or_default();
        usage.tokens_today += tokens;
        if agent_run {
            usage.agent_runs += 1;
            usage.concurrent_agents += 1;
        }
        Ok(())
    }

    /// Mark an agent run as completed (decrements concurrent count).
    pub fn complete_agent_run(&mut self, tenant_id: &str) -> Result<(), SaaSError> {
        let usage = self
            .usage
            .get_mut(tenant_id)
            .ok_or(SaaSError::TenantNotFound)?;
        usage.concurrent_agents = usage.concurrent_agents.saturating_sub(1);
        Ok(())
    }

    // --- Internal JWT helpers ---

    fn generate_jwt(
        &self,
        tenant_id: &str,
        email: &str,
        now: u64,
    ) -> Result<String, SaaSError> {
        let claims = TenantClaims {
            tenant_id: tenant_id.to_string(),
            email: email.to_string(),
            issued_at: now,
            expires_at: now + self.jwt_expiry_secs,
        };

        let header = base64_encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload_json =
            serde_json::to_string(&claims).map_err(|e| SaaSError::InvalidJwt(e.to_string()))?;
        let payload = base64_encode(&payload_json);

        let header_payload = format!("{header}.{payload}");
        let signature = hmac_sha256(&self.jwt_secret, &header_payload);

        Ok(format!("{header_payload}.{signature}"))
    }
}

// --- Utility functions ---

/// Get current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Simple HMAC-SHA256 using the audit crate's SHA-256.
/// HMAC(K, m) = H((K' ^ opad) || H((K' ^ ipad) || m))
fn hmac_sha256(key: &str, message: &str) -> String {
    let key_bytes = key.as_bytes();

    // If key > 64 bytes, hash it first.
    let key_block: Vec<u8> = if key_bytes.len() > 64 {
        let hashed = hash_api_key(key);
        hex_to_bytes(&hashed)
    } else {
        let mut k = key_bytes.to_vec();
        k.resize(64, 0);
        k
    };

    // Ensure key_block is exactly 64 bytes.
    let mut padded_key = key_block;
    padded_key.resize(64, 0);

    // ipad = key XOR 0x36, opad = key XOR 0x5c
    let mut ipad = vec![0u8; 64];
    let mut opad = vec![0u8; 64];
    for i in 0..64 {
        ipad[i] = padded_key[i] ^ 0x36;
        opad[i] = padded_key[i] ^ 0x5c;
    }

    // Inner hash: H(ipad || message)
    let mut inner_input = ipad;
    inner_input.extend_from_slice(message.as_bytes());
    let inner_hash_hex = hash_api_key(&String::from_utf8_lossy(&inner_input));

    // Outer hash: H(opad || inner_hash_bytes)
    let inner_hash_bytes = hex_to_bytes(&inner_hash_hex);
    let mut outer_input = opad;
    outer_input.extend_from_slice(&inner_hash_bytes);
    let outer_hash_hex = hash_api_key(&String::from_utf8_lossy(&outer_input));

    outer_hash_hex
}

/// Convert hex string to bytes.
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0))
        .collect()
}

/// Simple base64 encode (URL-safe, no padding).
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes = input.as_bytes();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 { u32::from(chunk[1]) } else { 0 };
        let b2 = if chunk.len() > 2 { u32::from(chunk[2]) } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }
    }

    result
}

/// Simple base64 decode (URL-safe, no padding).
fn base64_decode(input: &str) -> Result<String, String> {
    const DECODE: [u8; 128] = {
        let mut table = [255u8; 128];
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut i = 0;
        while i < 64 {
            table[alphabet[i] as usize] = i as u8;
            i += 1;
        }
        table
    };

    let bytes = input.as_bytes();
    let mut result = Vec::new();

    let mut i = 0;
    while i < bytes.len() {
        let remaining = bytes.len() - i;

        let b0 = decode_char(bytes[i], &DECODE)?;
        let b1 = if i + 1 < bytes.len() {
            decode_char(bytes[i + 1], &DECODE)?
        } else {
            0
        };
        let b2 = if i + 2 < bytes.len() {
            decode_char(bytes[i + 2], &DECODE)?
        } else {
            0
        };
        let b3 = if i + 3 < bytes.len() {
            decode_char(bytes[i + 3], &DECODE)?
        } else {
            0
        };

        let triple =
            (u32::from(b0) << 18) | (u32::from(b1) << 12) | (u32::from(b2) << 6) | u32::from(b3);

        result.push((triple >> 16) as u8);
        if remaining > 2 {
            result.push((triple >> 8) as u8);
        }
        if remaining > 3 {
            result.push(triple as u8);
        }

        i += 4.min(remaining);
        // If remaining < 4, we've consumed everything.
        if remaining < 4 {
            break;
        }
    }

    String::from_utf8(result).map_err(|e| format!("invalid utf-8: {e}"))
}

fn decode_char(c: u8, table: &[u8; 128]) -> Result<u8, String> {
    if c >= 128 || table[c as usize] == 255 {
        return Err(format!("invalid base64 character: {}", c as char));
    }
    Ok(table[c as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_platform() -> SaaSPlatform {
        SaaSPlatform::new("test-secret-key")
    }

    #[test]
    fn signup_creates_tenant_with_required_fields() {
        let mut platform = make_platform();
        let (tenant, token) = platform
            .signup("alice@example.com", "hashed_pw")
            .unwrap();

        assert!(!tenant.id.is_empty());
        assert!(tenant.id.starts_with("tenant-"));
        assert!(!tenant.workspace_dir.as_os_str().is_empty());
        assert!(tenant
            .workspace_dir
            .to_str()
            .unwrap()
            .contains(&tenant.id));
        assert!(!tenant.ollama_endpoint.is_empty());
        assert!(tenant.created_at > 0);
        assert_eq!(tenant.resource_limits.max_concurrent_agents, 4);
        assert!(!token.is_empty());
        // Token has 3 parts
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn signup_duplicate_email_rejected() {
        let mut platform = make_platform();
        platform.signup("bob@example.com", "pw1").unwrap();
        let result = platform.signup("bob@example.com", "pw2");
        assert_eq!(result.unwrap_err(), SaaSError::DuplicateEmail);
    }

    #[test]
    fn authenticate_valid_credentials() {
        let mut platform = make_platform();
        platform.signup("carol@example.com", "secret").unwrap();
        let token = platform.authenticate("carol@example.com", "secret").unwrap();
        assert!(!token.is_empty());
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn authenticate_wrong_password() {
        let mut platform = make_platform();
        platform.signup("dave@example.com", "correct").unwrap();
        let result = platform.authenticate("dave@example.com", "wrong");
        assert_eq!(result.unwrap_err(), SaaSError::AuthenticationFailed);
    }

    #[test]
    fn authenticate_unknown_email() {
        let platform = make_platform();
        let result = platform.authenticate("nobody@example.com", "pw");
        assert_eq!(result.unwrap_err(), SaaSError::AuthenticationFailed);
    }

    #[test]
    fn jwt_round_trip() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("eve@example.com", "pw").unwrap();
        let token = platform.authenticate("eve@example.com", "pw").unwrap();
        let claims = platform.validate_jwt(&token).unwrap();

        assert_eq!(claims.tenant_id, tenant.id);
        assert_eq!(claims.email, "eve@example.com");
        assert!(claims.expires_at > claims.issued_at);
        assert_eq!(
            claims.expires_at - claims.issued_at,
            DEFAULT_JWT_EXPIRY_SECS
        );
    }

    #[test]
    fn validate_expired_jwt() {
        // Use a platform with 0-second expiry so tokens are immediately expired.
        let mut platform = SaaSPlatform::with_expiry("secret", 0);
        platform.signup("frank@example.com", "pw").unwrap();
        let token = platform.authenticate("frank@example.com", "pw").unwrap();

        // The token was issued with expires_at = now + 0, so it's already expired
        // (or at the boundary). Sleep briefly to ensure expiry.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        let result = platform.validate_jwt(&token);
        assert!(matches!(result, Err(SaaSError::InvalidJwt(ref msg)) if msg.contains("expired")));
    }

    #[test]
    fn validate_jwt_invalid_signature() {
        let mut platform = make_platform();
        platform.signup("grace@example.com", "pw").unwrap();
        let token = platform.authenticate("grace@example.com", "pw").unwrap();

        // Tamper with the signature.
        let tampered = format!("{}.tampered", &token[..token.rfind('.').unwrap()]);
        let result = platform.validate_jwt(&tampered);
        assert!(
            matches!(result, Err(SaaSError::InvalidJwt(ref msg)) if msg.contains("signature"))
        );
    }

    #[test]
    fn validate_jwt_malformed() {
        let platform = make_platform();
        let result = platform.validate_jwt("not-a-jwt");
        assert!(matches!(result, Err(SaaSError::InvalidJwt(_))));
    }

    #[test]
    fn check_limits_within_bounds() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("heidi@example.com", "pw").unwrap();
        // No usage recorded yet — should be within limits.
        assert!(platform.check_limits(&tenant.id, "agent_run").is_ok());
    }

    #[test]
    fn check_limits_concurrent_agents_exceeded() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("ivan@example.com", "pw").unwrap();

        // Record enough agent runs to hit the limit (default: 4).
        for _ in 0..4 {
            platform.record_usage(&tenant.id, 0, true).unwrap();
        }

        let result = platform.check_limits(&tenant.id, "agent_run");
        assert!(matches!(
            result,
            Err(SaaSError::ResourceLimitExceeded(ref msg)) if msg.contains("concurrent")
        ));
    }

    #[test]
    fn check_limits_tokens_exceeded() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("judy@example.com", "pw").unwrap();

        // Exceed token limit.
        platform
            .record_usage(&tenant.id, 1_000_001, false)
            .unwrap();

        let result = platform.check_limits(&tenant.id, "query");
        assert!(matches!(
            result,
            Err(SaaSError::ResourceLimitExceeded(ref msg)) if msg.contains("tokens")
        ));
    }

    #[test]
    fn check_limits_tenant_not_found() {
        let platform = make_platform();
        let result = platform.check_limits("nonexistent", "agent_run");
        assert_eq!(result.unwrap_err(), SaaSError::TenantNotFound);
    }

    #[test]
    fn dashboard_returns_summary() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("kate@example.com", "pw").unwrap();

        // Record some usage.
        platform.record_usage(&tenant.id, 500, true).unwrap();
        platform.record_usage(&tenant.id, 300, true).unwrap();

        let summary = platform.dashboard(&tenant.id).unwrap();
        assert_eq!(summary.tenant_id, tenant.id);
        assert_eq!(summary.agent_runs, 2);
        assert_eq!(summary.token_consumption, 800);
        assert_eq!(summary.active_members, 1);
        assert_eq!(summary.billing_status, "active");
    }

    #[test]
    fn dashboard_tenant_not_found() {
        let platform = make_platform();
        let result = platform.dashboard("nonexistent");
        assert_eq!(result.unwrap_err(), SaaSError::TenantNotFound);
    }

    #[test]
    fn tenant_isolation_separate_workspaces() {
        let mut platform = make_platform();
        let (t1, _) = platform.signup("user1@example.com", "pw1").unwrap();
        let (t2, _) = platform.signup("user2@example.com", "pw2").unwrap();

        // Different tenant IDs.
        assert_ne!(t1.id, t2.id);
        // Different workspace directories.
        assert_ne!(t1.workspace_dir, t2.workspace_dir);
        // Each workspace dir contains its own tenant ID.
        assert!(t1.workspace_dir.to_str().unwrap().contains(&t1.id));
        assert!(t2.workspace_dir.to_str().unwrap().contains(&t2.id));
    }

    #[test]
    fn tenant_isolation_usage_separate() {
        let mut platform = make_platform();
        let (t1, _) = platform.signup("a@example.com", "pw").unwrap();
        let (t2, _) = platform.signup("b@example.com", "pw").unwrap();

        platform.record_usage(&t1.id, 1000, true).unwrap();

        let d1 = platform.dashboard(&t1.id).unwrap();
        let d2 = platform.dashboard(&t2.id).unwrap();

        assert_eq!(d1.token_consumption, 1000);
        assert_eq!(d1.agent_runs, 1);
        // Tenant 2 should have zero usage.
        assert_eq!(d2.token_consumption, 0);
        assert_eq!(d2.agent_runs, 0);
    }

    #[test]
    fn complete_agent_run_decrements_concurrent() {
        let mut platform = make_platform();
        let (tenant, _) = platform.signup("z@example.com", "pw").unwrap();

        platform.record_usage(&tenant.id, 0, true).unwrap();
        platform.record_usage(&tenant.id, 0, true).unwrap();
        // 2 concurrent agents.
        platform.complete_agent_run(&tenant.id).unwrap();
        // Now 1 concurrent agent — should still be within limits (max 4).
        assert!(platform.check_limits(&tenant.id, "agent_run").is_ok());
    }
}
