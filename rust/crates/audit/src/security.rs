//! Security utilities — hashing, rate limiting, input sanitization.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Hash an API key using the same SHA-256 from our audit chain.
/// Never store raw API keys — always hash them.
#[must_use] pub fn hash_api_key(key: &str) -> String {
    sha256_hex(key)
}

/// Verify an API key against a stored hash.
#[must_use] pub fn verify_api_key(key: &str, stored_hash: &str) -> bool {
    hash_api_key(key) == stored_hash
}

/// Simple rate limiter — tracks requests per IP/key within a time window.
pub struct RateLimiter {
    /// Maximum requests per window
    max_requests: u32,
    /// Time window duration
    window: Duration,
    /// Tracking: key → (count, `window_start`)
    entries: BTreeMap<String, (u32, Instant)>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_secs),
            entries: BTreeMap::new(),
        }
    }

    /// Check if a request is allowed. Returns true if allowed, false if rate limited.
    pub fn check(&mut self, key: &str) -> bool {
        let now = Instant::now();

        if let Some((count, start)) = self.entries.get_mut(key) {
            if now.duration_since(*start) > self.window {
                // Window expired, reset
                *count = 1;
                *start = now;
                true
            } else if *count >= self.max_requests {
                false
            } else {
                *count += 1;
                true
            }
        } else {
            self.entries.insert(key.to_string(), (1, now));
            true
        }
    }

    /// Clean up expired entries to prevent memory growth.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, (_, start)| now.duration_since(*start) <= self.window);
    }
}

/// Sanitize user input to prevent prompt injection attacks.
/// Removes control characters and limits length.
#[must_use] pub fn sanitize_prompt(input: &str, max_length: usize) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(max_length)
        .collect();
    cleaned.trim().to_string()
}

/// Check if a file path is safe (no directory traversal).
#[must_use] pub fn is_safe_path(path: &str) -> bool {
    !path.contains("..") && !path.starts_with('/') && !path.starts_with('~')
}

/// Redact sensitive patterns from text before logging.
#[must_use] pub fn redact_sensitive(text: &str) -> String {
    let mut redacted = text.to_string();

    // Redact common secret patterns
    let patterns = [
        // API keys (long alphanumeric strings after common prefixes)
        ("sk-", 20),
        ("pk-", 20),
        ("api_key=", 20),
        ("token=", 20),
        ("password=", 20),
        ("secret=", 20),
        ("ANTHROPIC_API_KEY=", 20),
        ("OPENAI_API_KEY=", 20),
    ];

    for (prefix, redact_len) in &patterns {
        if let Some(pos) = redacted.find(prefix) {
            let start = pos + prefix.len();
            let end = (start + redact_len).min(redacted.len());
            let replacement = "*".repeat(end - start);
            redacted.replace_range(start..end, &replacement);
        }
    }

    redacted
}

// SHA-256 (reuse from event.rs — but keep it here for the security module)
fn sha256_hex(input: &str) -> String {
    let bytes = input.as_bytes();
    let hash = sha256_bytes(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[allow(clippy::unreadable_literal)]
fn sha256_bytes(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (message.len() as u64) * 8;
    let mut padded = message.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 { padded.push(0); }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(temp1);
            d = c; c = b; b = a; a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut result = [0u8; 32];
    for (i, val) in h.iter().enumerate() {
        result[i*4..i*4+4].copy_from_slice(&val.to_be_bytes());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_api_key_is_deterministic() {
        let h1 = hash_api_key("my-secret-key");
        let h2 = hash_api_key("my-secret-key");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 = 64 hex chars
    }

    #[test]
    fn verify_api_key_works() {
        let hash = hash_api_key("test-key");
        assert!(verify_api_key("test-key", &hash));
        assert!(!verify_api_key("wrong-key", &hash));
    }

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut limiter = RateLimiter::new(3, 60);
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1")); // 4th request blocked
        assert!(limiter.check("user2")); // different user OK
    }

    #[test]
    fn sanitize_prompt_removes_control_chars() {
        let input = "hello\x00world\x01test";
        let clean = sanitize_prompt(input, 1000);
        assert_eq!(clean, "helloworldtest");
    }

    #[test]
    fn sanitize_prompt_preserves_newlines() {
        let input = "line1\nline2\ttab";
        let clean = sanitize_prompt(input, 1000);
        assert_eq!(clean, "line1\nline2\ttab");
    }

    #[test]
    fn sanitize_prompt_limits_length() {
        let input = "a".repeat(10000);
        let clean = sanitize_prompt(&input, 100);
        assert_eq!(clean.len(), 100);
    }

    #[test]
    fn is_safe_path_blocks_traversal() {
        assert!(!is_safe_path("../../../etc/passwd"));
        assert!(!is_safe_path("/etc/passwd"));
        assert!(!is_safe_path("~/secrets"));
        assert!(is_safe_path("src/main.rs"));
        assert!(is_safe_path("crates/runtime/src/lib.rs"));
    }

    #[test]
    fn redact_sensitive_hides_keys() {
        let text = "Using OPENAI_API_KEY=sk-abc123def456ghi789 for auth";
        let redacted = redact_sensitive(text);
        assert!(!redacted.contains("abc123"));
        assert!(redacted.contains("****"));
    }

    #[test]
    fn redact_sensitive_preserves_normal_text() {
        let text = "This is a normal message about code review";
        let redacted = redact_sensitive(text);
        assert_eq!(text, redacted);
    }
}
