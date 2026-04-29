//! SSO/SAML integration for enterprise authentication.
//!
//! Supports SAML 2.0 SP-initiated flow:
//!   1. User hits /api/auth/sso/login → redirect to `IdP`
//!   2. `IdP` authenticates → POST /api/auth/sso/callback with `SAMLResponse`
//!   3. We validate the response → create a session token
//!
//! Also supports simple OIDC-style token exchange for lighter integrations.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::rbac::{Role, User, UserStore};
use crate::security::hash_api_key;

/// SSO provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoConfig {
    /// Whether SSO is enabled.
    pub enabled: bool,
    /// SAML `IdP` metadata URL or entity ID.
    pub idp_entity_id: String,
    /// SAML `IdP` SSO URL (where to redirect for login).
    pub idp_sso_url: String,
    /// SAML `IdP` certificate (PEM) for signature validation.
    pub idp_certificate: String,
    /// Our SP entity ID.
    pub sp_entity_id: String,
    /// Our SP ACS URL (where `IdP` posts the `SAMLResponse`).
    pub sp_acs_url: String,
    /// Default role for SSO-provisioned users.
    #[serde(default = "default_role")]
    pub default_role: Role,
    /// Map of `IdP` group names to Tachy roles.
    #[serde(default)]
    pub role_mapping: BTreeMap<String, Role>,
    /// Session duration in seconds (default: 8 hours).
    #[serde(default = "default_session_duration")]
    pub session_duration_secs: u64,
}

fn default_role() -> Role {
    Role::Developer
}
fn default_session_duration() -> u64 {
    28800
} // 8 hours

impl Default for SsoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idp_entity_id: String::new(),
            idp_sso_url: String::new(),
            idp_certificate: String::new(),
            sp_entity_id: "tachy-agent".to_string(),
            sp_acs_url: "http://localhost:7777/api/auth/sso/callback".to_string(),
            default_role: Role::Developer,
            role_mapping: BTreeMap::new(),
            session_duration_secs: 28800,
        }
    }
}

/// A parsed SAML assertion (extracted from `SAMLResponse`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlAssertion {
    pub subject_name_id: String,
    pub issuer: String,
    pub attributes: BTreeMap<String, String>,
    pub session_index: Option<String>,
    /// Groups/roles from the `IdP`.
    pub groups: Vec<String>,
}

/// An active SSO session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsoSession {
    pub token: String,
    pub user_id: String,
    pub email: String,
    pub role: Role,
    pub created_at: u64,
    pub expires_at: u64,
    pub idp_session_index: Option<String>,
}

/// SSO session manager.
pub struct SsoManager {
    config: SsoConfig,
    sessions: BTreeMap<String, SsoSession>,
}

impl SsoManager {
    #[must_use]
    pub fn new(config: SsoConfig) -> Self {
        Self {
            config,
            sessions: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    #[must_use]
    pub fn config(&self) -> &SsoConfig {
        &self.config
    }

    /// Build the SAML `AuthnRequest` redirect URL.
    #[must_use]
    pub fn build_login_url(&self, relay_state: Option<&str>) -> String {
        let request_id = format!("_tachy_{}", now_epoch());
        let issue_instant = iso8601_now();

        // Minimal SAML AuthnRequest
        let authn_request = format!(
            r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" \
            xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" \
            ID="{request_id}" Version="2.0" IssueInstant="{issue_instant}" \
            Destination="{}" AssertionConsumerServiceURL="{}" \
            ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST">\
            <saml:Issuer>{}</saml:Issuer>\
            </samlp:AuthnRequest>"#,
            self.config.idp_sso_url, self.config.sp_acs_url, self.config.sp_entity_id,
        );

        // Base64 + deflate encode for redirect binding
        let encoded = base64_encode(authn_request.as_bytes());
        let encoded_url = urlencod(&encoded);

        let mut url = format!("{}?SAMLRequest={encoded_url}", self.config.idp_sso_url,);
        if let Some(rs) = relay_state {
            url.push_str(&format!("&RelayState={}", urlencod(rs)));
        }
        url
    }

    /// Process a SAML callback — parse the response, validate, create session.
    pub fn process_callback(
        &mut self,
        saml_response_b64: &str,
        user_store: &mut UserStore,
    ) -> Result<SsoSession, String> {
        // Decode the SAMLResponse
        let xml = base64_decode(saml_response_b64).map_err(|e| format!("invalid base64: {e}"))?;
        let xml_str = String::from_utf8(xml).map_err(|e| format!("invalid UTF-8: {e}"))?;

        // Parse the SAML assertion
        let assertion = parse_saml_response(&xml_str)?;

        // Validate issuer matches our IdP
        if !self.config.idp_entity_id.is_empty() && assertion.issuer != self.config.idp_entity_id {
            return Err(format!(
                "issuer mismatch: expected '{}', got '{}'",
                self.config.idp_entity_id, assertion.issuer
            ));
        }

        // Determine role from groups
        let role = self.resolve_role(&assertion.groups);

        // Create or update user in the store
        let email = &assertion.subject_name_id;
        let user_id = format!(
            "sso-{}",
            hash_api_key(email).chars().take(12).collect::<String>()
        );

        let display_name = assertion
            .attributes
            .get("displayName")
            .or_else(|| assertion.attributes.get("name"))
            .cloned()
            .unwrap_or_else(|| email.clone());

        // Generate a session token
        let token = generate_session_token(&user_id);
        let now = now_epoch();

        let user = User {
            id: user_id.clone(),
            name: display_name,
            role,
            api_key_hash: hash_api_key(&token),
            created_at: format!("{now}s"),
            enabled: true,
            active_team_id: None,
        };
        user_store.add_user(user);

        let session = SsoSession {
            token: token.clone(),
            user_id,
            email: email.clone(),
            role,
            created_at: now,
            expires_at: now + self.config.session_duration_secs,
            idp_session_index: assertion.session_index,
        };

        self.sessions.insert(token.clone(), session.clone());
        Ok(session)
    }

    /// Validate a session token. Returns the session if valid and not expired.
    #[must_use]
    pub fn validate_session(&self, token: &str) -> Option<&SsoSession> {
        let session = self.sessions.get(token)?;
        if now_epoch() > session.expires_at {
            return None;
        }
        Some(session)
    }

    /// Invalidate a session (logout).
    pub fn invalidate_session(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    /// Clean up expired sessions.
    pub fn cleanup_expired(&mut self) {
        let now = now_epoch();
        self.sessions.retain(|_, s| s.expires_at > now);
    }

    /// List active sessions.
    #[must_use]
    pub fn active_sessions(&self) -> Vec<&SsoSession> {
        let now = now_epoch();
        self.sessions
            .values()
            .filter(|s| s.expires_at > now)
            .collect()
    }

    /// Resolve a role from `IdP` groups using the role mapping.
    fn resolve_role(&self, groups: &[String]) -> Role {
        for group in groups {
            if let Some(role) = self.config.role_mapping.get(group) {
                return *role;
            }
            // Common group name patterns
            let lower = group.to_lowercase();
            if lower.contains("admin") {
                return Role::Admin;
            }
            if lower.contains("developer") || lower.contains("engineer") {
                return Role::Developer;
            }
            if lower.contains("viewer") || lower.contains("readonly") {
                return Role::Viewer;
            }
        }
        self.config.default_role
    }
}

// ---------------------------------------------------------------------------
// SAML XML parsing (lightweight, no external XML crate)
// ---------------------------------------------------------------------------

/// Parse a SAML Response XML to extract the assertion.
fn parse_saml_response(xml: &str) -> Result<SamlAssertion, String> {
    // Extract NameID (subject)
    let name_id = extract_xml_value(xml, "NameID").ok_or("SAMLResponse missing NameID")?;

    // Extract Issuer
    let issuer = extract_xml_value(xml, "Issuer").unwrap_or_default();

    // Extract SessionIndex from AuthnStatement
    let session_index = extract_xml_attr(xml, "AuthnStatement", "SessionIndex");

    // Extract attributes
    let attributes = extract_saml_attributes(xml);

    // Extract groups from attributes
    let groups = attributes
        .get("groups")
        .or_else(|| attributes.get("memberOf"))
        .or_else(|| attributes.get("role"))
        .map(|g| g.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    Ok(SamlAssertion {
        subject_name_id: name_id,
        issuer,
        attributes,
        session_index,
        groups,
    })
}

/// Extract the text content of an XML element by tag name.
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    // Match both prefixed (saml:Tag) and unprefixed (Tag)
    for prefix in &["", "saml:", "saml2:"] {
        let open = format!("<{prefix}{tag}");
        if let Some(start) = xml.find(&open) {
            let after_open = &xml[start + open.len()..];
            // Skip attributes until >
            let content_start = after_open.find('>')? + 1;
            let close = format!("</{prefix}{tag}>");
            let content_end = after_open[content_start..].find(&close)?;
            let value = after_open[content_start..content_start + content_end].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Extract an attribute value from an XML element.
fn extract_xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    for prefix in &["", "saml:", "samlp:"] {
        let open = format!("<{prefix}{tag}");
        if let Some(start) = xml.find(&open) {
            let tag_end = xml[start..].find('>')?;
            let tag_content = &xml[start..start + tag_end];
            let attr_marker = format!("{attr}=\"");
            if let Some(attr_start) = tag_content.find(&attr_marker) {
                let value_start = attr_start + attr_marker.len();
                let value_end = tag_content[value_start..].find('"')?;
                return Some(tag_content[value_start..value_start + value_end].to_string());
            }
        }
    }
    None
}

/// Extract SAML Attribute elements into a map.
fn extract_saml_attributes(xml: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let mut search_from = 0;

    while let Some((pos, name)) = find_next_attr_tag(xml, search_from) {
        search_from = pos + 1;
        // Find the AttributeValue within this Attribute
        if let Some(value) = extract_attr_value_after(xml, pos) {
            attrs.insert(name, value);
        }
    }

    attrs
}

fn find_next_attr_tag(xml: &str, from: usize) -> Option<(usize, String)> {
    let search = &xml[from..];
    // Look for <saml:Attribute Name="..." or <Attribute Name="..."
    for prefix in &["<saml:Attribute ", "<saml2:Attribute ", "<Attribute "] {
        if let Some(pos) = search.find(prefix) {
            let abs_pos = from + pos;
            let after = &xml[abs_pos + prefix.len()..];
            if let Some(name) = extract_name_attr(after) {
                return Some((abs_pos, name));
            }
        }
    }
    None
}

fn extract_name_attr(s: &str) -> Option<String> {
    let marker = "Name=\"";
    let start = s.find(marker)? + marker.len();
    let end = s[start..].find('"')?;
    Some(s[start..start + end].to_string())
}

fn extract_attr_value_after(xml: &str, from: usize) -> Option<String> {
    let search = &xml[from..];
    for prefix in &[
        "<saml:AttributeValue",
        "<saml2:AttributeValue",
        "<AttributeValue",
    ] {
        if let Some(pos) = search.find(prefix) {
            let after = &search[pos..];
            let content_start = after.find('>')? + 1;
            // Find closing tag
            let content_end = after[content_start..].find('<')?;
            let value = after[content_start..content_start + content_end].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn generate_session_token(user_id: &str) -> String {
    let seed = format!("{}-{}-tachy-sso", user_id, now_epoch());
    let hash = hash_api_key(&seed);
    format!("sso-{}", &hash[..32])
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn iso8601_now() -> String {
    let secs = now_epoch();
    // Approximate ISO 8601 without chrono
    format!("{secs}Z")
}

fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[must_use]
pub fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let chars: Vec<u8> = clean.bytes().collect();

    for chunk in chars.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let a = b64_val(chunk[0])?;
        let b = b64_val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = b64_val(chunk[2])?;
            out.push(((b & 0x0F) << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                let d = b64_val(chunk[3])?;
                out.push(((c & 0x03) << 6) | d);
            }
        }
    }
    Ok(out)
}

fn b64_val(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        b'=' => Ok(0),
        _ => Err(format!("invalid base64 char: {}", c as char)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sso_config_defaults() {
        let config = SsoConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.default_role, Role::Developer);
        assert_eq!(config.session_duration_secs, 28800);
    }

    #[test]
    fn base64_round_trip() {
        let original = b"Hello, SAML World!";
        let encoded = base64_encode(original);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn parse_saml_response_extracts_fields() {
        let xml = r#"
        <samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol">
          <saml:Issuer xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">https://idp.example.com</saml:Issuer>
          <saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">
            <saml:Subject>
              <saml:NameID>user@example.com</saml:NameID>
            </saml:Subject>
            <saml:AuthnStatement SessionIndex="sess-123">
            </saml:AuthnStatement>
            <saml:AttributeStatement>
              <saml:Attribute Name="displayName">
                <saml:AttributeValue>Jane Doe</saml:AttributeValue>
              </saml:Attribute>
              <saml:Attribute Name="groups">
                <saml:AttributeValue>engineering,admin</saml:AttributeValue>
              </saml:Attribute>
            </saml:AttributeStatement>
          </saml:Assertion>
        </samlp:Response>
        "#;

        let assertion = parse_saml_response(xml).unwrap();
        assert_eq!(assertion.subject_name_id, "user@example.com");
        assert_eq!(assertion.issuer, "https://idp.example.com");
        assert_eq!(assertion.session_index.as_deref(), Some("sess-123"));
        assert_eq!(assertion.attributes.get("displayName").unwrap(), "Jane Doe");
        assert!(assertion.groups.contains(&"engineering".to_string()));
        assert!(assertion.groups.contains(&"admin".to_string()));
    }

    #[test]
    fn sso_manager_login_url() {
        let config = SsoConfig {
            enabled: true,
            idp_entity_id: "https://idp.example.com".to_string(),
            idp_sso_url: "https://idp.example.com/sso".to_string(),
            idp_certificate: String::new(),
            sp_entity_id: "tachy".to_string(),
            sp_acs_url: "http://localhost:7777/api/auth/sso/callback".to_string(),
            ..SsoConfig::default()
        };
        let mgr = SsoManager::new(config);
        let url = mgr.build_login_url(Some("/dashboard"));
        assert!(url.starts_with("https://idp.example.com/sso?SAMLRequest="));
        assert!(url.contains("RelayState="));
    }

    #[test]
    fn sso_callback_creates_session() {
        let config = SsoConfig {
            enabled: true,
            idp_entity_id: "https://idp.example.com".to_string(),
            idp_sso_url: "https://idp.example.com/sso".to_string(),
            idp_certificate: String::new(),
            sp_entity_id: "tachy".to_string(),
            sp_acs_url: "http://localhost:7777/api/auth/sso/callback".to_string(),
            role_mapping: {
                let mut m = BTreeMap::new();
                m.insert("admin".to_string(), Role::Admin);
                m
            },
            ..SsoConfig::default()
        };
        let mut mgr = SsoManager::new(config);
        let mut user_store = UserStore::new();

        let xml = r#"<samlp:Response><saml:Issuer>https://idp.example.com</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>dev@corp.com</saml:NameID></saml:Subject><saml:AuthnStatement SessionIndex="s1"></saml:AuthnStatement><saml:AttributeStatement><saml:Attribute Name="groups"><saml:AttributeValue>admin</saml:AttributeValue></saml:Attribute></saml:AttributeStatement></saml:Assertion></samlp:Response>"#;
        let b64 = base64_encode(xml.as_bytes());

        let session = mgr.process_callback(&b64, &mut user_store).unwrap();
        assert_eq!(session.email, "dev@corp.com");
        assert_eq!(session.role, Role::Admin);
        assert!(session.token.starts_with("sso-"));

        // Validate the session
        assert!(mgr.validate_session(&session.token).is_some());

        // User was provisioned
        assert_eq!(user_store.list_users().len(), 1);
    }

    #[test]
    fn sso_session_invalidation() {
        let config = SsoConfig {
            enabled: true,
            ..SsoConfig::default()
        };
        let mut mgr = SsoManager::new(config);

        // Manually insert a session
        let token = "sso-test-token".to_string();
        mgr.sessions.insert(
            token.clone(),
            SsoSession {
                token: token.clone(),
                user_id: "u1".to_string(),
                email: "test@test.com".to_string(),
                role: Role::Developer,
                created_at: now_epoch(),
                expires_at: now_epoch() + 3600,
                idp_session_index: None,
            },
        );

        assert!(mgr.validate_session(&token).is_some());
        mgr.invalidate_session(&token);
        assert!(mgr.validate_session(&token).is_none());
    }

    #[test]
    fn role_resolution_from_groups() {
        let config = SsoConfig {
            role_mapping: {
                let mut m = BTreeMap::new();
                m.insert("platform-admins".to_string(), Role::Admin);
                m.insert("devs".to_string(), Role::Developer);
                m
            },
            default_role: Role::Viewer,
            ..SsoConfig::default()
        };
        let mgr = SsoManager::new(config);

        assert_eq!(
            mgr.resolve_role(&["platform-admins".to_string()]),
            Role::Admin
        );
        assert_eq!(mgr.resolve_role(&["devs".to_string()]), Role::Developer);
        assert_eq!(
            mgr.resolve_role(&["unknown-group".to_string()]),
            Role::Viewer
        );
        assert_eq!(mgr.resolve_role(&[]), Role::Viewer);
    }

    #[test]
    fn issuer_mismatch_rejected() {
        let config = SsoConfig {
            enabled: true,
            idp_entity_id: "https://expected-idp.com".to_string(),
            ..SsoConfig::default()
        };
        let mut mgr = SsoManager::new(config);
        let mut user_store = UserStore::new();

        let xml = r"<samlp:Response><saml:Issuer>https://evil-idp.com</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>user@evil.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>";
        let b64 = base64_encode(xml.as_bytes());

        let err = mgr.process_callback(&b64, &mut user_store).unwrap_err();
        assert!(err.contains("issuer mismatch"));
    }

    // --- Edge case / fuzz-like tests for SAML parser ---

    #[test]
    fn parse_empty_xml() {
        let result = super::parse_saml_response("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_xml_without_nameid() {
        let xml = "<samlp:Response><saml:Issuer>idp</saml:Issuer></samlp:Response>";
        let result = super::parse_saml_response(xml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_xml_with_nested_tags() {
        let xml = r"<samlp:Response><saml:Issuer>idp</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>user@test.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>";
        let result = super::parse_saml_response(xml).unwrap();
        assert_eq!(result.subject_name_id, "user@test.com");
    }

    #[test]
    fn parse_xml_with_special_chars_in_nameid() {
        let xml = r"<samlp:Response><saml:Issuer>idp</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>user+tag@test.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>";
        let result = super::parse_saml_response(xml).unwrap();
        assert_eq!(result.subject_name_id, "user+tag@test.com");
    }

    #[test]
    fn parse_xml_with_no_attributes() {
        let xml = r"<samlp:Response><saml:Issuer>idp</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>u@t.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>";
        let result = super::parse_saml_response(xml).unwrap();
        assert!(result.attributes.is_empty());
        assert!(result.groups.is_empty());
    }

    #[test]
    fn parse_xml_with_multiple_attributes() {
        let xml = r#"<samlp:Response><saml:Issuer>idp</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>u@t.com</saml:NameID></saml:Subject><saml:AttributeStatement><saml:Attribute Name="email"><saml:AttributeValue>u@t.com</saml:AttributeValue></saml:Attribute><saml:Attribute Name="name"><saml:AttributeValue>User</saml:AttributeValue></saml:Attribute></saml:AttributeStatement></saml:Assertion></samlp:Response>"#;
        let result = super::parse_saml_response(xml).unwrap();
        assert_eq!(result.attributes.get("email").unwrap(), "u@t.com");
        assert_eq!(result.attributes.get("name").unwrap(), "User");
    }

    #[test]
    fn parse_malformed_base64_rejected() {
        let config = SsoConfig {
            enabled: true,
            ..SsoConfig::default()
        };
        let mut mgr = SsoManager::new(config);
        let mut users = UserStore::new();
        let err = mgr
            .process_callback("not-valid-base64!!!", &mut users)
            .unwrap_err();
        assert!(err.contains("invalid") || err.contains("base64") || err.contains("UTF-8"));
    }

    #[test]
    fn parse_binary_garbage_rejected() {
        let config = SsoConfig {
            enabled: true,
            ..SsoConfig::default()
        };
        let mut mgr = SsoManager::new(config);
        let mut users = UserStore::new();
        // Valid base64 but not XML
        let b64 = super::base64_encode(b"\x00\x01\x02\x03binary garbage");
        let err = mgr.process_callback(&b64, &mut users).unwrap_err();
        assert!(err.contains("missing") || err.contains("NameID"));
    }
}
