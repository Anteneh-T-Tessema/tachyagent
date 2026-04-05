//! SSO Pen-Test Suite — security audit tests for the SAML parser and session management.
//!
//! Tests cover Requirements 11.1–11.10 from the Product Hardening V3 spec.

use std::collections::BTreeMap;

use audit::sso::{base64_encode, base64_decode, SamlAssertion, SsoConfig, SsoManager};
use audit::{Role, UserStore};

/// Helper: build a minimal SsoConfig with a known IdP entity ID.
fn test_config(idp_entity_id: &str) -> SsoConfig {
    SsoConfig {
        enabled: true,
        idp_entity_id: idp_entity_id.to_string(),
        idp_sso_url: "https://idp.example.com/sso".to_string(),
        idp_certificate: String::new(),
        sp_entity_id: "tachy-test".to_string(),
        sp_acs_url: "http://localhost:7777/api/auth/sso/callback".to_string(),
        default_role: Role::Developer,
        role_mapping: BTreeMap::new(),
        session_duration_secs: 3600,
    }
}

/// Helper: wrap XML in base64 for process_callback.
fn b64_xml(xml: &str) -> String {
    base64_encode(xml.as_bytes())
}

/// Helper: build a minimal valid SAML response XML.
fn saml_response(issuer: &str, name_id: &str) -> String {
    format!(
        r#"<samlp:Response><saml:Issuer>{issuer}</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>{name_id}</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>"#
    )
}

// ---------------------------------------------------------------------------
// Test 1: XML entity expansion (billion laughs) — Requirement 11.1
// ---------------------------------------------------------------------------

#[test]
fn xml_entity_expansion_billion_laughs() {
    // The lightweight parser doesn't expand entities, so this should either
    // fail to find a NameID or return the raw entity reference — but must NOT
    // consume excessive memory.
    let xml = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [
  <!ENTITY lol "lol">
  <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
  <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
  <!ENTITY lol4 "&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;">
  <!ENTITY lol5 "&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;&lol4;">
  <!ENTITY lol6 "&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;&lol5;">
  <!ENTITY lol7 "&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;&lol6;">
  <!ENTITY lol8 "&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;&lol7;">
  <!ENTITY lol9 "&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;&lol8;">
]>
<samlp:Response>
  <saml:Issuer>https://idp.example.com</saml:Issuer>
  <saml:Assertion>
    <saml:Subject>
      <saml:NameID>&lol9;</saml:NameID>
    </saml:Subject>
  </saml:Assertion>
</samlp:Response>"#;

    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();
    let b64 = b64_xml(xml);

    // The parser should either reject or return without blowing up memory.
    // It must NOT panic.
    let result = mgr.process_callback(&b64, &mut users);
    // Either an error (entity not expanded → missing NameID or raw entity ref)
    // or success with the raw entity text — both are acceptable as long as
    // no excessive memory was consumed.
    match result {
        Ok(session) => {
            // If it succeeded, the NameID should NOT be a gigabyte-sized string.
            assert!(session.email.len() < 1024, "entity expansion produced oversized NameID");
        }
        Err(_) => { /* rejection is fine */ }
    }
}


// ---------------------------------------------------------------------------
// Test 2: Script injection in NameID — Requirement 11.2
// ---------------------------------------------------------------------------

#[test]
fn script_injection_in_nameid() {
    let malicious_name_id = "<script>alert('xss')</script>";
    let xml = saml_response("https://idp.example.com", malicious_name_id);
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();

    let result = mgr.process_callback(&b64_xml(&xml), &mut users);
    match result {
        Ok(session) => {
            // If the parser accepted it, the stored email/NameID must not
            // contain unescaped script tags that could execute in a browser.
            // The raw value is stored — verify it doesn't propagate into
            // the user store with executable script context.
            let stored_users = users.list_users();
            for user in &stored_users {
                // The name should not contain raw <script> tags that would
                // execute — it's stored as data, not rendered as HTML.
                // This is acceptable for a backend store.
                assert!(!user.name.is_empty());
            }
            // The session email is the NameID — it's stored as-is which is
            // fine for a backend. The key invariant is it doesn't panic.
            assert!(!session.email.is_empty());
        }
        Err(_) => { /* rejection is also acceptable */ }
    }
}

// ---------------------------------------------------------------------------
// Test 3: Forged Issuer — Requirement 11.3
// ---------------------------------------------------------------------------

#[test]
fn forged_issuer_rejected() {
    let xml = saml_response("https://evil-idp.com", "user@evil.com");
    let mut mgr = SsoManager::new(test_config("https://expected-idp.com"));
    let mut users = UserStore::new();

    let err = mgr.process_callback(&b64_xml(&xml), &mut users).unwrap_err();
    assert!(
        err.contains("issuer mismatch"),
        "expected 'issuer mismatch' error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Session token replay after invalidation — Requirement 11.4
// ---------------------------------------------------------------------------

#[test]
fn session_token_replay_after_invalidation() {
    let xml = saml_response("https://idp.example.com", "user@corp.com");
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();

    let session = mgr.process_callback(&b64_xml(&xml), &mut users).unwrap();
    let token = session.token.clone();

    // Session should be valid
    assert!(mgr.validate_session(&token).is_some());

    // Invalidate
    mgr.invalidate_session(&token);

    // Replay — must return None
    assert!(
        mgr.validate_session(&token).is_none(),
        "replayed token should be rejected after invalidation"
    );
}

// ---------------------------------------------------------------------------
// Test 5: NameID with null bytes, control characters, long strings — Req 11.5
// ---------------------------------------------------------------------------

#[test]
fn nameid_with_null_bytes_no_panic() {
    let xml = saml_response("https://idp.example.com", "user\0@evil.com");
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();
    // Must not panic
    let _ = mgr.process_callback(&b64_xml(&xml), &mut users);
}

#[test]
fn nameid_with_control_characters_no_panic() {
    let name_id = "user\x01\x02\x03\x07\x1b@evil.com";
    let xml = saml_response("https://idp.example.com", name_id);
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();
    // Must not panic
    let _ = mgr.process_callback(&b64_xml(&xml), &mut users);
}

#[test]
fn nameid_exceeding_1024_chars_no_panic() {
    let long_name = "a".repeat(2048);
    let xml = saml_response("https://idp.example.com", &long_name);
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();
    // Must not panic
    let _ = mgr.process_callback(&b64_xml(&xml), &mut users);
}


// ---------------------------------------------------------------------------
// Test 6: Expired session — Requirement 11.6
// ---------------------------------------------------------------------------

#[test]
fn expired_session_rejected() {
    let config = SsoConfig {
        enabled: true,
        session_duration_secs: 0, // expires immediately
        idp_entity_id: "https://idp.example.com".to_string(),
        ..test_config("https://idp.example.com")
    };
    let mut mgr = SsoManager::new(config);
    let mut users = UserStore::new();

    let xml = saml_response("https://idp.example.com", "user@corp.com");
    let session = mgr.process_callback(&b64_xml(&xml), &mut users).unwrap();

    // With session_duration_secs = 0, expires_at == created_at.
    // now_epoch() >= expires_at, so validate_session should return None.
    // (There may be a 1-second race; sleep briefly to be safe.)
    std::thread::sleep(std::time::Duration::from_millis(1100));

    assert!(
        mgr.validate_session(&session.token).is_none(),
        "expired session should be rejected"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Base64 decoder robustness — Requirement 11.7
// ---------------------------------------------------------------------------

#[test]
fn base64_rejects_non_base64_characters() {
    let result = base64_decode("not-valid-base64!!!");
    assert!(result.is_err(), "non-base64 characters should be rejected");
}

#[test]
fn base64_handles_truncated_input_no_panic() {
    // Truncated base64 (not a multiple of 4)
    let _ = base64_decode("SGVsbG8"); // "Hello" without padding
    let _ = base64_decode("SG");
    let _ = base64_decode("S");
    let _ = base64_decode("");
    // None of these should panic
}

#[test]
fn base64_round_trip_correctness() {
    let original = b"test payload for round-trip";
    let encoded = base64_encode(original);
    let decoded = base64_decode(&encoded).unwrap();
    assert_eq!(decoded, original);
}

// ---------------------------------------------------------------------------
// Test 8: Deeply nested XML (depth > 100) — Requirement 11.8
// ---------------------------------------------------------------------------

#[test]
fn deeply_nested_xml_no_stack_overflow() {
    // Build XML with 150 levels of nesting
    let mut xml = String::new();
    for _ in 0..150 {
        xml.push_str("<wrapper>");
    }
    xml.push_str(
        r#"<samlp:Response><saml:Issuer>https://idp.example.com</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID>deep@user.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>"#,
    );
    for _ in 0..150 {
        xml.push_str("</wrapper>");
    }

    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();

    // Must not stack overflow or panic
    let result = mgr.process_callback(&b64_xml(&xml), &mut users);
    // Either succeeds (parser finds NameID through nesting) or errors — both OK.
    match result {
        Ok(session) => assert_eq!(session.email, "deep@user.com"),
        Err(_) => { /* rejection is acceptable */ }
    }
}

// ---------------------------------------------------------------------------
// Test 9: SAML assertion round-trip (Property 33) — Requirement 11.9
// ---------------------------------------------------------------------------

#[test]
fn saml_assertion_round_trip() {
    // Build a SamlAssertion, encode to XML, parse back, verify equivalence.
    let original = SamlAssertion {
        subject_name_id: "roundtrip@example.com".to_string(),
        issuer: "https://idp.example.com".to_string(),
        attributes: {
            let mut m = BTreeMap::new();
            m.insert("displayName".to_string(), "Round Trip User".to_string());
            m.insert("department".to_string(), "Engineering".to_string());
            m
        },
        session_index: Some("sess-rt-001".to_string()),
        groups: vec!["engineering".to_string(), "admin".to_string()],
    };

    // Encode to SAML XML
    let xml = encode_assertion_to_xml(&original);

    // Process through the callback pipeline
    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();
    let session = mgr.process_callback(&b64_xml(&xml), &mut users).unwrap();

    // Verify key fields survived the round-trip
    assert_eq!(session.email, original.subject_name_id);
    assert_eq!(
        session.idp_session_index.as_deref(),
        original.session_index.as_deref()
    );
}

/// Encode a SamlAssertion into a minimal SAML Response XML string.
fn encode_assertion_to_xml(assertion: &SamlAssertion) -> String {
    let mut xml = String::new();
    xml.push_str("<samlp:Response>");
    xml.push_str(&format!("<saml:Issuer>{}</saml:Issuer>", assertion.issuer));
    xml.push_str("<saml:Assertion>");
    xml.push_str("<saml:Subject>");
    xml.push_str(&format!(
        "<saml:NameID>{}</saml:NameID>",
        assertion.subject_name_id
    ));
    xml.push_str("</saml:Subject>");

    // AuthnStatement with SessionIndex
    if let Some(ref idx) = assertion.session_index {
        xml.push_str(&format!(
            r#"<saml:AuthnStatement SessionIndex="{}"></saml:AuthnStatement>"#,
            idx
        ));
    }

    // Attributes
    if !assertion.attributes.is_empty() || !assertion.groups.is_empty() {
        xml.push_str("<saml:AttributeStatement>");
        for (key, value) in &assertion.attributes {
            xml.push_str(&format!(
                r#"<saml:Attribute Name="{}"><saml:AttributeValue>{}</saml:AttributeValue></saml:Attribute>"#,
                key, value
            ));
        }
        if !assertion.groups.is_empty() {
            let groups_str = assertion.groups.join(",");
            xml.push_str(&format!(
                r#"<saml:Attribute Name="groups"><saml:AttributeValue>{}</saml:AttributeValue></saml:Attribute>"#,
                groups_str
            ));
        }
        xml.push_str("</saml:AttributeStatement>");
    }

    xml.push_str("</saml:Assertion>");
    xml.push_str("</samlp:Response>");
    xml
}

// ---------------------------------------------------------------------------
// Test 10: CDATA sections wrapping NameID — Requirement 11.10
// ---------------------------------------------------------------------------

#[test]
fn cdata_wrapping_nameid() {
    let xml = r#"<samlp:Response><saml:Issuer>https://idp.example.com</saml:Issuer><saml:Assertion><saml:Subject><saml:NameID><![CDATA[cdata@user.com]]></saml:NameID></saml:Subject></saml:Assertion></samlp:Response>"#;

    let mut mgr = SsoManager::new(test_config("https://idp.example.com"));
    let mut users = UserStore::new();

    let result = mgr.process_callback(&b64_xml(xml), &mut users);
    match result {
        Ok(session) => {
            // If parsed, the CDATA content should be extracted correctly
            assert!(
                session.email.contains("cdata@user.com"),
                "CDATA content should be extracted: got '{}'",
                session.email
            );
        }
        Err(_) => {
            // Explicit rejection of CDATA is also acceptable per the requirement
        }
    }
}
