use serde::{Deserialize, Serialize};

/// Trait for asymmetric cryptographic signing (e.g. Ed25519).
/// Used by the audit logger to authenticate events.
pub trait AsymmetricSigner: Send + Sync {
    /// Sign a message and return (`HexSignature`, `HexPublicKey`).
    fn sign_payload(&self, message: &[u8]) -> (String, String);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventKind {
    SessionStart,
    SessionEnd,
    UserMessage,
    AssistantMessage,
    ToolInvocation,
    ToolResult,
    PermissionGranted,
    PermissionDenied,
    GovernanceViolation,
    SessionCompacted,
    ConfigChange,
    ModelSwitch,
    UsageMetering,
    RoleChange,
    VisualSnapshot,
    ResourceCleanup,
    /// Autonomous repair attempt (Phase 25: The Healer).
    SelfRepair,
    /// Multi-agent consensus approval (Phase 27).
    ConsensusSeal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: String,
    pub session_id: String,
    pub kind: AuditEventKind,
    pub severity: AuditSeverity,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    pub tool_name: Option<String>,
    pub model_name: Option<String>,
    pub detail: String,
    /// Redacted input/output for compliance — never store raw sensitive data.
    pub redacted_payload: Option<String>,
    /// Sequence number for ordering within the audit trail.
    #[serde(default)]
    pub sequence: u64,
    /// SHA-256 hash of this event's content + previous hash (hash chain).
    /// If any event is modified or deleted, the chain breaks.
    #[serde(default)]
    pub hash: String,
    /// Hash of the previous event in the chain. Empty for the first event.
    #[serde(default)]
    pub prev_hash: String,
    /// Cryptographic signature of the hash (asymmetric signing).
    pub signature: Option<String>,
    /// Hex-encoded public key of the signing agent.
    pub public_key: Option<String>,
    /// Optional path to a visual snapshot related to this event.
    pub visual_anchor: Option<String>,
    /// Optional: structured metadata for visual verification (Phase 26).
    /// Stores diff similarity, viewport info, etc.
    pub visual_metadata: Option<serde_json::Value>,
    /// Optional: structured consensus report from swarm governance (Phase 27).
    pub consensus_report: Option<serde_json::Value>,
}

impl AuditEvent {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        kind: AuditEventKind,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: now_iso8601(),
            session_id: session_id.into(),
            kind,
            severity: AuditSeverity::Info,
            agent_id: None,
            user_id: None,
            tool_name: None,
            model_name: None,
            detail: detail.into(),
            redacted_payload: None,
            sequence: 0,
            hash: String::new(),
            prev_hash: String::new(),
            signature: None,
            public_key: None,
            visual_anchor: None,
            visual_metadata: None,
            consensus_report: None,
        }
    }

    #[must_use]
    pub fn with_severity(mut self, severity: AuditSeverity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    #[must_use]
    pub fn with_model(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = Some(model_name.into());
        self
    }

    #[must_use]
    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    #[must_use]
    pub fn with_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    #[must_use]
    pub fn with_redacted_payload(mut self, payload: impl Into<String>) -> Self {
        self.redacted_payload = Some(payload.into());
        self
    }

    #[must_use]
    pub fn with_visual_anchor(mut self, path: impl Into<String>) -> Self {
        self.visual_anchor = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_visual_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.visual_metadata = Some(metadata);
        self
    }

    #[must_use]
    pub fn with_consensus_report(mut self, report: serde_json::Value) -> Self {
        self.consensus_report = Some(report);
        self
    }

    #[must_use]
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"error\":\"serialize_failed\",\"detail\":\"{}\"}}",
                self.detail
            )
        })
    }

    /// Compute the content string used for hashing (excludes hash fields).
    fn hash_content(&self) -> String {
        format!(
            "{}|{}|{:?}|{:?}|{}|{}|{}|{}|{}",
            self.timestamp,
            self.session_id,
            self.kind,
            self.severity,
            self.detail,
            self.sequence,
            self.visual_anchor.as_deref().unwrap_or(""),
            self.visual_metadata
                .as_ref()
                .map(std::string::ToString::to_string)
                .unwrap_or_default(),
            self.consensus_report
                .as_ref()
                .map(std::string::ToString::to_string)
                .unwrap_or_default(),
        )
    }

    /// Sign this event with a hash chain. Sets sequence, `prev_hash`, and hash.
    pub fn sign(&mut self, sequence: u64, prev_hash: &str) {
        self.sequence = sequence;
        self.prev_hash = prev_hash.to_string();
        let content = self.hash_content();
        self.hash = sha256_hex(&format!("{prev_hash}|{content}"));
    }

    /// Verify this event's hash is correct given the previous hash.
    #[must_use]
    pub fn verify(&self, prev_hash: &str) -> bool {
        let content = self.hash_content();
        let expected = sha256_hex(&format!("{prev_hash}|{content}"));
        self.hash == expected && self.prev_hash == prev_hash
    }

    /// Verify the asymmetric cryptographic signature.
    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let (Some(sig_hex), Some(pk_hex)) = (&self.signature, &self.public_key) else {
            return false;
        };

        let Ok(sig_bytes) = hex_to_bytes(sig_hex) else {
            return false;
        };
        let Ok(pk_bytes) = hex_to_bytes(pk_hex) else {
            return false;
        };

        let Ok(signature) = ed25519_dalek::Signature::from_slice(&sig_bytes) else {
            return false;
        };
        let Ok(public_key) =
            ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes.try_into().unwrap_or([0u8; 32]))
        else {
            return false;
        };

        use ed25519_dalek::Verifier;
        public_key.verify(self.hash.as_bytes(), &signature).is_ok()
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Verify an entire audit chain. Returns Ok(count) or Err(first broken index).
pub fn verify_audit_chain(events: &[AuditEvent]) -> Result<usize, usize> {
    let mut prev_hash = String::new();
    for (i, event) in events.iter().enumerate() {
        if !event.verify(&prev_hash) {
            return Err(i);
        }
        prev_hash = event.hash.clone();
    }
    Ok(events.len())
}

/// Simple SHA-256 implementation (no external crate).
/// Uses the standard algorithm from FIPS 180-4.
fn sha256_hex(input: &str) -> String {
    let bytes = input.as_bytes();
    let hash = sha256_bytes(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Public wrapper for use by the license module.
pub fn sha256_hex_public(input: &str) -> String {
    sha256_hex(input)
}

/// Public wrapper for use by the license module.
pub fn sha256_bytes_public(message: &[u8]) -> [u8; 32] {
    sha256_bytes(message)
}

#[allow(clippy::unreadable_literal)]
fn sha256_bytes(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Padding
    let bit_len = (message.len() as u64) * 8;
    let mut padded = message.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process blocks
    for chunk in padded.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut result = [0u8; 32];
    for (i, val) in h.iter().enumerate() {
        result[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    result
}

fn now_iso8601() -> String {
    // Simple timestamp without external crate — seconds since epoch
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_audit_event_with_builder() {
        let event = AuditEvent::new("sess-1", AuditEventKind::ToolInvocation, "executed bash")
            .with_severity(AuditSeverity::Warning)
            .with_tool("bash")
            .with_model("qwen2.5-coder:7b")
            .with_agent("code-reviewer")
            .with_user("user-42");

        assert_eq!(event.session_id, "sess-1");
        assert_eq!(event.tool_name.as_deref(), Some("bash"));
        assert_eq!(event.severity, AuditSeverity::Warning);

        let json = event.to_json_line();
        assert!(json.contains("tool_invocation"));
        assert!(json.contains("bash"));
    }
}
