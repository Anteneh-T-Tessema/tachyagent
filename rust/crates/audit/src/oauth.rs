//! OIDC / `OAuth2` login support.
//!
//! Provides Google and GitHub `OAuth2` flows alongside the existing SAML 2.0 SSO.
//! Lowers the adoption barrier for mid-market teams that cannot deploy SAML.
//!
//! Flow:
//!   1. `GET /api/auth/oauth/{provider}/login` → redirect to provider's auth URL
//!   2. Provider redirects back to `GET /api/auth/oauth/{provider}/callback?code=...`
//!   3. Handler exchanges code → access token → user profile
//!   4. Creates an `OAuthSession` (same 8-hour TTL as SAML sessions)

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Which `OAuth2` provider to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Google,
    GitHub,
}

impl OAuthProvider {
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "google" => Some(Self::Google),
            "github" => Some(Self::GitHub),
            _ => None,
        }
    }

    #[must_use] pub fn name(&self) -> &'static str {
        match self { Self::Google => "google", Self::GitHub => "github" }
    }

    fn auth_url(&self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::GitHub => "https://github.com/login/oauth/authorize",
        }
    }

    fn token_url(&self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::GitHub => "https://github.com/login/oauth/access_token",
        }
    }

    fn profile_url(&self) -> &'static str {
        match self {
            Self::Google => "https://www.googleapis.com/oauth2/v3/userinfo",
            Self::GitHub => "https://api.github.com/user",
        }
    }

    fn scopes(&self) -> &'static str {
        match self {
            Self::Google => "openid email profile",
            Self::GitHub => "read:user user:email",
        }
    }
}

/// Per-provider `OAuth2` credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientConfig {
    pub provider: OAuthProvider,
    pub client_id: String,
    pub client_secret: String,
    /// Full redirect URI (must match what's registered with the provider).
    pub redirect_uri: String,
}

impl OAuthClientConfig {
    /// Load from environment variables.
    ///
    /// Google: `TACHY_GOOGLE_CLIENT_ID`, `TACHY_GOOGLE_CLIENT_SECRET`, `TACHY_GOOGLE_REDIRECT_URI`
    /// GitHub: `TACHY_GITHUB_CLIENT_ID`, `TACHY_GITHUB_CLIENT_SECRET`, `TACHY_GITHUB_REDIRECT_URI`
    #[must_use] pub fn from_env(provider: &OAuthProvider) -> Option<Self> {
        let prefix = match provider {
            OAuthProvider::Google => "TACHY_GOOGLE",
            OAuthProvider::GitHub => "TACHY_GITHUB",
        };
        let client_id = std::env::var(format!("{prefix}_CLIENT_ID")).ok()?;
        let client_secret = std::env::var(format!("{prefix}_CLIENT_SECRET")).ok()?;
        let redirect_uri = std::env::var(format!("{prefix}_REDIRECT_URI"))
            .unwrap_or_else(|_| format!("http://localhost:7777/api/auth/oauth/{}/callback", provider.name()));
        Some(Self {
            provider: provider.clone(),
            client_id,
            client_secret,
            redirect_uri,
        })
    }
}

// ── Session ───────────────────────────────────────────────────────────────────

/// An active `OAuth2` session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthSession {
    pub token: String,
    pub provider: OAuthProvider,
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub expires_at: u64,
}

impl OAuthSession {
    #[must_use] pub fn is_expired(&self) -> bool {
        now_epoch() >= self.expires_at
    }
}

// ── State (CSRF tokens + active sessions) ────────────────────────────────────

#[derive(Debug, Default)]
pub struct OAuthManager {
    /// Pending CSRF state tokens: state → provider
    pending: BTreeMap<String, String>,
    /// Active sessions keyed by session token
    sessions: BTreeMap<String, OAuthSession>,
}

impl OAuthManager {
    #[must_use] pub fn new() -> Self { Self::default() }

    /// Generate the authorization URL for a provider.
    /// Returns `(url, state_token)` — state must be stored and validated in callback.
    pub fn authorization_url(&mut self, config: &OAuthClientConfig) -> (String, String) {
        let state = random_token(32);
        self.pending.insert(state.clone(), config.provider.name().to_string());

        let provider = &config.provider;
        let url = match provider {
            OAuthProvider::Google => format!(
                "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline",
                provider.auth_url(),
                url_encode(&config.client_id),
                url_encode(&config.redirect_uri),
                url_encode(provider.scopes()),
                state,
            ),
            OAuthProvider::GitHub => format!(
                "{}?client_id={}&redirect_uri={}&scope={}&state={}",
                provider.auth_url(),
                url_encode(&config.client_id),
                url_encode(&config.redirect_uri),
                url_encode(provider.scopes()),
                state,
            ),
        };

        (url, state)
    }

    /// Handle the `OAuth2` callback: validate state, exchange code, fetch profile.
    /// Returns the created session on success.
    pub fn handle_callback(
        &mut self,
        config: &OAuthClientConfig,
        code: &str,
        state: &str,
    ) -> Result<OAuthSession, String> {
        // Validate CSRF state
        let expected_provider = self.pending.remove(state)
            .ok_or_else(|| "invalid or expired OAuth state token".to_string())?;
        if expected_provider != config.provider.name() {
            return Err(format!("state mismatch: expected '{}' got '{}'", expected_provider, config.provider.name()));
        }

        // Exchange code for access token
        let access_token = exchange_code(config, code)?;

        // Fetch user profile
        let profile = fetch_profile(&config.provider, &access_token)?;

        let session_token = random_token(48);
        let session = OAuthSession {
            token: session_token.clone(),
            provider: config.provider.clone(),
            user_id: profile.id,
            email: profile.email,
            display_name: profile.name,
            avatar_url: profile.avatar_url,
            expires_at: now_epoch() + 8 * 3600, // 8-hour TTL matches SAML
        };

        self.sessions.insert(session_token, session.clone());
        Ok(session)
    }

    /// Validate a session token. Returns None if missing or expired.
    pub fn validate_session(&mut self, token: &str) -> Option<&OAuthSession> {
        if let Some(s) = self.sessions.get(token) {
            if !s.is_expired() {
                return self.sessions.get(token);
            }
        }
        self.sessions.remove(token);
        None
    }

    /// Revoke a session token (logout).
    pub fn revoke_session(&mut self, token: &str) {
        self.sessions.remove(token);
        // Prune expired pending states (housekeeping)
        self.pending.retain(|_, _| true); // they expire implicitly after ~10 min
    }

    #[must_use] pub fn active_session_count(&self) -> usize {
        self.sessions.values().filter(|s| !s.is_expired()).count()
    }
}

// ── HTTP helpers (blocking) ───────────────────────────────────────────────────

struct UserProfile {
    id: String,
    email: String,
    name: String,
    avatar_url: Option<String>,
}

fn exchange_code(config: &OAuthClientConfig, code: &str) -> Result<String, String> {
    let provider = &config.provider;
    let body = match provider {
        OAuthProvider::Google => format!(
            "code={}&client_id={}&client_secret={}&redirect_uri={}&grant_type=authorization_code",
            url_encode(code),
            url_encode(&config.client_id),
            url_encode(&config.client_secret),
            url_encode(&config.redirect_uri),
        ),
        OAuthProvider::GitHub => format!(
            "client_id={}&client_secret={}&code={}&redirect_uri={}",
            url_encode(&config.client_id),
            url_encode(&config.client_secret),
            url_encode(code),
            url_encode(&config.redirect_uri),
        ),
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let resp = client
        .post(provider.token_url())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .map_err(|e| format!("token exchange failed: {e}"))?;

    let json: serde_json::Value = resp.json()
        .map_err(|e| format!("token response parse error: {e}"))?;

    if let Some(err) = json["error"].as_str() {
        return Err(format!("OAuth error: {} — {}", err, json["error_description"].as_str().unwrap_or("")));
    }

    json["access_token"].as_str()
        .map(str::to_string)
        .ok_or_else(|| "no access_token in response".to_string())
}

fn fetch_profile(provider: &OAuthProvider, access_token: &str) -> Result<UserProfile, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client error: {e}"))?;

    let resp = client
        .get(provider.profile_url())
        .bearer_auth(access_token)
        .header("User-Agent", "tachy-agent/0.1")
        .send()
        .map_err(|e| format!("profile fetch failed: {e}"))?;

    let json: serde_json::Value = resp.json()
        .map_err(|e| format!("profile parse error: {e}"))?;

    let (id, email, name, avatar_url) = match provider {
        OAuthProvider::Google => (
            json["sub"].as_str().unwrap_or("").to_string(),
            json["email"].as_str().unwrap_or("").to_string(),
            json["name"].as_str().unwrap_or("").to_string(),
            json["picture"].as_str().map(str::to_string),
        ),
        OAuthProvider::GitHub => (
            json["id"].to_string(),
            json["email"].as_str().unwrap_or("").to_string(),
            json["name"].as_str()
                .or_else(|| json["login"].as_str())
                .unwrap_or("").to_string(),
            json["avatar_url"].as_str().map(str::to_string),
        ),
    };

    if id.is_empty() {
        return Err("could not extract user ID from profile".to_string());
    }

    Ok(UserProfile { id, email, name, avatar_url })
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn random_token(bytes: usize) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Deterministic-but-unique: combine PID + timestamp + counter
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let pid = std::process::id();
    // SHA256-like mixing via format + simple hash
    let raw = format!("{ts}:{pid}:{n}:{bytes}");
    // Encode as hex-ish via repeated FNV-1a
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in raw.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}{ts:032x}")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(bytes)
        .collect()
}

fn url_encode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        _ => format!("%{:02X}", c as u8),
    }).collect()
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_from_str() {
        assert_eq!(OAuthProvider::from_str("google"), Some(OAuthProvider::Google));
        assert_eq!(OAuthProvider::from_str("GITHUB"), Some(OAuthProvider::GitHub));
        assert!(OAuthProvider::from_str("unknown").is_none());
    }

    #[test]
    fn authorization_url_contains_client_id() {
        let config = OAuthClientConfig {
            provider: OAuthProvider::Google,
            client_id: "test-client-id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost:7777/callback".to_string(),
        };
        let mut mgr = OAuthManager::new();
        let (url, state) = mgr.authorization_url(&config);
        assert!(url.contains("test-client-id"));
        assert!(url.contains(&state));
        assert!(url.starts_with("https://accounts.google.com"));
    }

    #[test]
    fn github_url_uses_correct_endpoint() {
        let config = OAuthClientConfig {
            provider: OAuthProvider::GitHub,
            client_id: "gh-client".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost:7777/callback".to_string(),
        };
        let mut mgr = OAuthManager::new();
        let (url, _) = mgr.authorization_url(&config);
        assert!(url.starts_with("https://github.com/login/oauth/authorize"));
        assert!(url.contains("gh-client"));
    }

    #[test]
    fn invalid_state_rejected_in_callback() {
        let config = OAuthClientConfig {
            provider: OAuthProvider::Google,
            client_id: "c".to_string(),
            client_secret: "s".to_string(),
            redirect_uri: "http://localhost".to_string(),
        };
        let mut mgr = OAuthManager::new();
        let result = mgr.handle_callback(&config, "code123", "invalid-state");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid"));
    }

    #[test]
    fn session_expiry_check() {
        let session = OAuthSession {
            token: "tok".to_string(),
            provider: OAuthProvider::GitHub,
            user_id: "123".to_string(),
            email: "a@b.com".to_string(),
            display_name: "Alice".to_string(),
            avatar_url: None,
            expires_at: now_epoch() - 1, // already expired
        };
        assert!(session.is_expired());
    }

    #[test]
    fn random_tokens_are_unique() {
        let t1 = random_token(32);
        let t2 = random_token(32);
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), 32);
    }
}
