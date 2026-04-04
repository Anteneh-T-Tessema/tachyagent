//! License management for Tachy.
//!
//! Supports:
//! - 7-day free trial (no key needed)
//! - Signed license keys for paid activation
//! - Offline verification using embedded public key
//! - Machine-bound licenses (optional)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many seconds in 7 days.
const TRIAL_DURATION_SECS: u64 = 7 * 24 * 60 * 60;

/// License tiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseTier {
    /// Free trial — 7 days from first run.
    Trial,
    /// Individual developer license.
    Individual,
    /// Team license (up to N seats).
    Team { seats: u32 },
    /// Enterprise site license.
    Enterprise,
}

/// Current license status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseStatus {
    /// Trial is active, N seconds remaining.
    TrialActive { remaining_secs: u64 },
    /// Trial has expired.
    TrialExpired { expired_ago_secs: u64 },
    /// Paid license is active.
    Licensed { tier: LicenseTier, email: String },
    /// License file is corrupt or tampered.
    Invalid { reason: String },
}

impl LicenseStatus {
    /// Can the user run Tachy?
    pub fn is_active(&self) -> bool {
        matches!(self, Self::TrialActive { .. } | Self::Licensed { .. })
    }

    /// Human-readable status for display.
    pub fn display(&self) -> String {
        match self {
            Self::TrialActive { remaining_secs } => {
                let days = remaining_secs / 86400;
                let hours = (remaining_secs % 86400) / 3600;
                format!("Trial: {days}d {hours}h remaining")
            }
            Self::TrialExpired { expired_ago_secs } => {
                let days = expired_ago_secs / 86400;
                format!("Trial expired {days} days ago. Run `tachy activate <KEY>` to continue.")
            }
            Self::Licensed { tier, email } => {
                let tier_name = match tier {
                    LicenseTier::Trial => "Trial",
                    LicenseTier::Individual => "Individual",
                    LicenseTier::Team { seats } => return format!("Team ({seats} seats) — {email}"),
                    LicenseTier::Enterprise => "Enterprise",
                };
                format!("{tier_name} — {email}")
            }
            Self::Invalid { reason } => format!("Invalid license: {reason}"),
        }
    }
}

/// Persisted license file at `.tachy/license.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFile {
    /// When Tachy was first run (epoch seconds). Set once, never changed.
    pub first_run_at: u64,
    /// Machine ID — SHA-256 of hostname + username. For machine-bound licenses.
    pub machine_id: String,
    /// License key (empty during trial).
    #[serde(default)]
    pub license_key: String,
    /// Decoded license data (populated after activation).
    #[serde(default)]
    pub license: Option<LicenseData>,
}

/// Decoded and verified license payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseData {
    pub email: String,
    pub tier: LicenseTier,
    /// Expiry as epoch seconds (0 = perpetual).
    pub expires_at: u64,
    /// Optional machine ID binding.
    pub machine_id: Option<String>,
    /// Issue timestamp.
    pub issued_at: u64,
}

impl LicenseFile {
    /// Create a new trial license file.
    pub fn new_trial() -> Self {
        Self {
            first_run_at: now_epoch(),
            machine_id: compute_machine_id(),
            license_key: String::new(),
            license: None,
        }
    }

    /// Load from disk, or create a new trial if not found.
    pub fn load_or_create(tachy_dir: &Path) -> Self {
        let path = license_path(tachy_dir);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(file) = serde_json::from_str::<LicenseFile>(&content) {
                return file;
            }
        }
        // First run — create trial
        let file = Self::new_trial();
        let _ = file.save(tachy_dir);
        file
    }

    /// Save to disk.
    pub fn save(&self, tachy_dir: &Path) -> Result<(), String> {
        let path = license_path(tachy_dir);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize license: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("write license file: {e}"))
    }

    /// Check current license status.
    pub fn status(&self) -> LicenseStatus {
        // If we have a verified license, check it
        if let Some(license) = &self.license {
            // Check expiry
            if license.expires_at > 0 && now_epoch() > license.expires_at {
                return LicenseStatus::TrialExpired {
                    expired_ago_secs: now_epoch() - license.expires_at,
                };
            }
            // Check machine binding
            if let Some(bound_machine) = &license.machine_id {
                if *bound_machine != self.machine_id {
                    return LicenseStatus::Invalid {
                        reason: "license is bound to a different machine".to_string(),
                    };
                }
            }
            return LicenseStatus::Licensed {
                tier: license.tier.clone(),
                email: license.email.clone(),
            };
        }

        // No license — check trial
        let elapsed = now_epoch().saturating_sub(self.first_run_at);
        if elapsed < TRIAL_DURATION_SECS {
            LicenseStatus::TrialActive {
                remaining_secs: TRIAL_DURATION_SECS - elapsed,
            }
        } else {
            LicenseStatus::TrialExpired {
                expired_ago_secs: elapsed - TRIAL_DURATION_SECS,
            }
        }
    }

    /// Activate a license key. The key format is:
    /// `TACHY-<base64_payload>-<base64_signature>`
    ///
    /// The payload is JSON: `{"email":"...","tier":"individual","expires_at":0,"machine_id":null,"issued_at":...}`
    /// The signature is HMAC-SHA256 of the payload using the license secret.
    ///
    /// For v1, we use a shared secret (HMAC). For v2, switch to Ed25519 public key.
    pub fn activate(&mut self, key: &str, secret: &str) -> Result<LicenseData, String> {
        let parts: Vec<&str> = key.split('-').collect();
        if parts.len() != 3 || parts[0] != "TACHY" {
            return Err("invalid key format — expected TACHY-<payload>-<signature>".to_string());
        }

        let payload_b64 = parts[1];
        let sig_b64 = parts[2];

        // Decode payload
        let payload_bytes = base64_decode(payload_b64)
            .map_err(|_| "invalid key: payload decode failed".to_string())?;
        let payload_str = String::from_utf8(payload_bytes)
            .map_err(|_| "invalid key: payload is not UTF-8".to_string())?;

        // Verify signature (HMAC-SHA256)
        let expected_sig = hmac_sha256(secret.as_bytes(), payload_str.as_bytes());
        let provided_sig = base64_decode(sig_b64)
            .map_err(|_| "invalid key: signature decode failed".to_string())?;

        if expected_sig != provided_sig {
            return Err("invalid license key — signature verification failed".to_string());
        }

        // Parse license data
        let license: LicenseData = serde_json::from_str(&payload_str)
            .map_err(|e| format!("invalid license payload: {e}"))?;

        // Check expiry
        if license.expires_at > 0 && now_epoch() > license.expires_at {
            return Err("this license key has expired".to_string());
        }

        self.license_key = key.to_string();
        self.license = Some(license.clone());
        Ok(license)
    }
}

fn license_path(tachy_dir: &Path) -> PathBuf {
    tachy_dir.join("license.json")
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn compute_machine_id() -> String {
    let hostname = std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    // Use the same SHA-256 from event.rs
    let input = format!("{hostname}|{user}|tachy-machine-id");
    crate::event::sha256_hex_public(&input)
}

/// Simple base64 decode (no external crate).
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = Vec::new();
    let input = input.trim_end_matches('=');
    let bytes: Vec<u8> = input.bytes().collect();

    for chunk in bytes.chunks(4) {
        let mut buf = 0u32;
        let mut count = 0;
        for &b in chunk {
            let val = TABLE.iter().position(|&c| c == b).ok_or(())? as u32;
            buf = (buf << 6) | val;
            count += 1;
        }
        buf <<= (4 - count) * 6;
        let out_bytes = match count {
            4 => vec![(buf >> 16) as u8, (buf >> 8) as u8, buf as u8],
            3 => vec![(buf >> 16) as u8, (buf >> 8) as u8],
            2 => vec![(buf >> 16) as u8],
            _ => vec![],
        };
        output.extend(out_bytes);
    }
    Ok(output)
}

/// HMAC-SHA256 using the pure Rust SHA-256 from event.rs.
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    let block_size = 64;

    // If key is longer than block size, hash it
    let key = if key.len() > block_size {
        crate::event::sha256_bytes_public(key).to_vec()
    } else {
        key.to_vec()
    };

    // Pad key to block size
    let mut ipad = vec![0x36u8; block_size];
    let mut opad = vec![0x5cu8; block_size];
    for (i, &b) in key.iter().enumerate() {
        ipad[i] ^= b;
        opad[i] ^= b;
    }

    // Inner hash: SHA256(ipad || message)
    let mut inner = ipad;
    inner.extend_from_slice(message);
    let inner_hash = crate::event::sha256_bytes_public(&inner);

    // Outer hash: SHA256(opad || inner_hash)
    let mut outer = opad;
    outer.extend_from_slice(&inner_hash);
    crate::event::sha256_bytes_public(&outer).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_trial_is_active() {
        let file = LicenseFile::new_trial();
        let status = file.status();
        assert!(status.is_active());
        match status {
            LicenseStatus::TrialActive { remaining_secs } => {
                assert!(remaining_secs > TRIAL_DURATION_SECS - 10);
            }
            _ => panic!("expected TrialActive"),
        }
    }

    #[test]
    fn expired_trial_is_not_active() {
        let mut file = LicenseFile::new_trial();
        file.first_run_at = now_epoch() - TRIAL_DURATION_SECS - 100;
        assert!(!file.status().is_active());
    }

    #[test]
    fn activate_with_valid_key() {
        let secret = "test-secret-key-for-tachy";
        let payload = r#"{"email":"[email]","tier":"individual","expires_at":0,"machine_id":null,"issued_at":1700000000}"#;
        let sig = hmac_sha256(secret.as_bytes(), payload.as_bytes());
        let key = format!("TACHY-{}-{}", base64_encode(payload.as_bytes()), base64_encode(&sig));

        let mut file = LicenseFile::new_trial();
        let result = file.activate(&key, secret);
        assert!(result.is_ok());
        let license = result.unwrap();
        assert_eq!(license.tier, LicenseTier::Individual);
        assert!(file.status().is_active());
    }

    #[test]
    fn activate_with_wrong_secret_fails() {
        let secret = "correct-secret";
        let payload = r#"{"email":"[email]","tier":"individual","expires_at":0,"machine_id":null,"issued_at":1700000000}"#;
        let sig = hmac_sha256(b"wrong-secret", payload.as_bytes());
        let key = format!("TACHY-{}-{}", base64_encode(payload.as_bytes()), base64_encode(&sig));

        let mut file = LicenseFile::new_trial();
        let result = file.activate(&key, secret);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("signature"));
    }

    #[test]
    fn machine_id_is_deterministic() {
        let id1 = compute_machine_id();
        let id2 = compute_machine_id();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }

    #[test]
    fn base64_roundtrip() {
        let data = b"hello world";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    fn base64_encode(data: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
            result.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 { result.push(TABLE[((triple >> 6) & 0x3F) as usize] as char); } else { result.push('='); }
            if chunk.len() > 2 { result.push(TABLE[(triple & 0x3F) as usize] as char); } else { result.push('='); }
        }
        result
    }
}
