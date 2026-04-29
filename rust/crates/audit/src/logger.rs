use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::event::{sha256_bytes_public, AuditEvent};

/// Trait for audit log destinations — file, database, remote endpoint, etc.
pub trait AuditSink: Send + Sync {
    fn write_event(&self, event: &AuditEvent) -> Result<(), String>;
    fn flush(&self) -> Result<(), String>;
}

/// Append-only file-based audit sink. One JSON line per event.
pub struct FileAuditSink {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

impl FileAuditSink {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create audit dir: {e}"))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("failed to open audit log: {e}"))?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AuditSink for FileAuditSink {
    fn write_event(&self, event: &AuditEvent) -> Result<(), String> {
        let line = event.to_json_line();
        let mut file = self.file.lock().map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| format!("audit write failed: {e}"))
    }

    fn flush(&self) -> Result<(), String> {
        let mut file = self.file.lock().map_err(|e| e.to_string())?;
        file.flush().map_err(|e| format!("audit flush failed: {e}"))
    }
}

/// In-memory sink for testing.
#[derive(Default, Clone)]
#[allow(dead_code)]
pub struct MemoryAuditSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[allow(dead_code)]
impl MemoryAuditSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl AuditSink for MemoryAuditSink {
    fn write_event(&self, event: &AuditEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|e| e.to_string())?
            .push(event.clone());
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Main audit logger that dispatches to one or more sinks.
pub struct AuditLogger {
    sinks: Vec<Box<dyn AuditSink>>,
    sequence: Mutex<u64>,
    last_hash: Mutex<String>,
    /// Secret values that must be redacted from event details before writing.
    masked_secrets: Mutex<Vec<String>>,
}

impl AuditLogger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            sequence: Mutex::new(0),
            last_hash: Mutex::new(String::new()),
            masked_secrets: Mutex::new(Vec::new()),
        }
    }

    /// Register a secret value to be redacted from all future audit event details.
    /// Safe to call through `Arc<AuditLogger>` — uses interior mutability.
    pub fn mask_secret(&self, secret: &str) {
        if !secret.is_empty() {
            self.masked_secrets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(secret.to_string());
        }
    }

    /// Create a logger that continues the hash chain from an existing audit file.
    #[must_use]
    pub fn resume_from_file(path: &Path) -> Self {
        let logger = Self::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            let mut max_seq = 0u64;
            let mut last_hash = String::new();
            for line in content.lines() {
                if let Ok(event) = serde_json::from_str::<AuditEvent>(line) {
                    if event.sequence >= max_seq {
                        max_seq = event.sequence;
                        last_hash = event.hash.clone();
                    }
                }
            }
            *logger.sequence.lock().unwrap() = max_seq;
            *logger.last_hash.lock().unwrap() = last_hash;
        }
        logger
    }

    pub fn add_sink(&mut self, sink: impl AuditSink + 'static) {
        self.sinks.push(Box::new(sink));
    }

    /// Log an event with proper hash chain signing.
    pub fn log(&self, event: &AuditEvent) {
        self.log_signed(event, None);
    }

    /// Log an event with both hash chain signing and an asymmetric signature.
    pub fn log_signed(
        &self,
        event: &AuditEvent,
        signer: Option<&dyn crate::event::AsymmetricSigner>,
    ) {
        let mut seq = self
            .sequence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut last = self
            .last_hash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        *seq += 1;
        let mut signed = event.clone();
        signed.sign(*seq, &last);
        *last = signed.hash.clone();

        // Perform asymmetric signing of the final event hash if a signer is provided
        if let Some(signer) = signer {
            let (sig, pk) = signer.sign_payload(signed.hash.as_bytes());
            signed.signature = Some(sig);
            signed.public_key = Some(pk);
        }

        // Redact registered secrets from the detail field before persisting.
        {
            let masks = self
                .masked_secrets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for secret in masks.iter() {
                if signed.detail.contains(secret.as_str()) {
                    signed.detail = signed.detail.replace(secret.as_str(), "[REDACTED]");
                }
            }
        }

        for sink in &self.sinks {
            if let Err(e) = sink.write_event(&signed) {
                eprintln!("audit sink error: {e}");
            }
        }
    }

    pub fn flush(&self) {
        for sink in &self.sinks {
            let _ = sink.flush();
        }
    }

    /// Flush all sinks, collecting errors rather than silently dropping them.
    pub fn flush_with_errors(&self, errors: &mut Vec<String>) {
        for sink in &self.sinks {
            if let Err(e) = sink.flush() {
                errors.push(e);
            }
        }
    }

    /// Get the current sequence number.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        *self
            .sequence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Get the last hash in the chain.
    #[must_use]
    pub fn last_hash(&self) -> String {
        self.last_hash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

// ── HttpAuditSink ─────────────────────────────────────────────────────────────

/// Batching HTTP sink — POSTs audit events as JSON to any compatible endpoint.
///
/// # Compatible targets
/// - **`PostgREST`** → `PostgreSQL`: `http://localhost:3000/audit_events`
/// - **Splunk HEC**: `https://splunk.corp/services/collector/event`
/// - **Elastic / `OpenSearch`**: `https://elastic.corp/audit/_doc`
/// - **Datadog Logs API**: `https://http-intake.logs.datadoghq.com/api/v2/logs`
/// - Any REST endpoint that accepts `{"events": [...]}` or `[...]`
///
/// Events are buffered until `batch_size` is reached or `flush()` is called.
/// On failure the buffer is retained so no events are silently dropped.
pub struct HttpAuditSink {
    endpoint: String,
    bearer_token: Option<String>,
    /// Additional HTTP headers (e.g. `X-Splunk-Index`, `x-api-key`).
    extra_headers: Vec<(String, String)>,
    batch_size: usize,
    buffer: Mutex<Vec<AuditEvent>>,
    client: reqwest::blocking::Client,
}

impl HttpAuditSink {
    /// Create a new HTTP sink.
    ///
    /// `endpoint` — full URL to POST batches to.
    /// `bearer_token` — optional `Authorization: Bearer <token>` value.
    /// `batch_size` — flush automatically after this many events (default 50).
    pub fn new(
        endpoint: impl Into<String>,
        bearer_token: Option<String>,
        batch_size: Option<usize>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            bearer_token,
            extra_headers: Vec::new(),
            batch_size: batch_size.unwrap_or(50),
            buffer: Mutex::new(Vec::new()),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Add a custom header sent with every batch request.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }

    fn send_batch(&self, events: &[AuditEvent]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        let body = serde_json::json!({ "events": events });
        let mut req = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", "application/json");
        if let Some(token) = &self.bearer_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .json(&body)
            .send()
            .map_err(|e| format!("http audit send failed: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("http audit endpoint returned {}", resp.status()))
        }
    }
}

impl AuditSink for HttpAuditSink {
    fn write_event(&self, event: &AuditEvent) -> Result<(), String> {
        let should_flush = {
            let mut buf = self.buffer.lock().map_err(|e| e.to_string())?;
            buf.push(event.clone());
            buf.len() >= self.batch_size
        };
        if should_flush {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        let batch: Vec<AuditEvent> = {
            let mut buf = self.buffer.lock().map_err(|e| e.to_string())?;
            std::mem::take(&mut *buf)
        };
        if batch.is_empty() {
            return Ok(());
        }
        if let Err(e) = self.send_batch(&batch) {
            // Re-queue on failure so events are not lost
            let mut buf = self.buffer.lock().map_err(|e2| e2.to_string())?;
            let mut requeued = batch;
            requeued.append(&mut buf);
            *buf = requeued;
            return Err(e);
        }
        Ok(())
    }
}

// ── S3AuditSink ───────────────────────────────────────────────────────────────

/// Batching S3 sink — uploads JSONL bundles to any S3-compatible object store.
///
/// Each flush writes one object:
///   `<prefix>/YYYY/MM/DD/seq-<from>-<to>-<hash>.jsonl`
///
/// Objects are immutable once written — enabling WORM-style compliance storage.
///
/// # Compatible targets
/// - **AWS S3** (standard)
/// - **`MinIO`** (on-prem sovereign storage)
/// - **Cloudflare R2** / **Backblaze B2** / **`DigitalOcean` Spaces**
///
/// Authentication uses AWS Signature Version 4 (HMAC-SHA256).
/// No AWS SDK required — implemented with `reqwest::blocking`.
pub struct S3AuditSink {
    bucket: String,
    prefix: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    endpoint_override: Option<String>,
    batch_size: usize,
    buffer: Mutex<Vec<AuditEvent>>,
    client: reqwest::blocking::Client,
}

impl S3AuditSink {
    pub fn new(
        bucket: impl Into<String>,
        prefix: impl Into<String>,
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        batch_size: Option<usize>,
    ) -> Self {
        Self {
            bucket: bucket.into(),
            prefix: prefix.into(),
            region: region.into(),
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            endpoint_override: None,
            batch_size: batch_size.unwrap_or(100),
            buffer: Mutex::new(Vec::new()),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Override the S3 endpoint (required for `MinIO`, R2, Spaces, B2).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint_override = Some(endpoint.into());
        self
    }

    fn base_url(&self) -> String {
        if let Some(ep) = &self.endpoint_override {
            format!("{}/{}", ep.trim_end_matches('/'), self.bucket)
        } else {
            format!("https://{}.s3.{}.amazonaws.com", self.bucket, self.region)
        }
    }

    fn upload(&self, key: &str, body: &[u8]) -> Result<(), String> {
        let url = format!("{}/{}", self.base_url(), key);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let date_time = format_iso8601(now.as_secs());
        let date_only = &date_time[..8];
        let content_sha256 = hex_encode(&sha256_bytes_public(body));
        let content_len = body.len().to_string();

        // Canonical request
        let canonical_headers = format!(
            "content-length:{}\ncontent-type:application/x-ndjson\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n",
            content_len,
            self.host(),
            content_sha256,
            date_time,
        );
        let signed_headers = "content-length;content-type;host;x-amz-content-sha256;x-amz-date";
        let canonical_request =
            format!("PUT\n/{key}\n\n{canonical_headers}\n{signed_headers}\n{content_sha256}");

        // String to sign
        let scope = format!("{date_only}/{}/s3/aws4_request", self.region);
        let cr_hash = hex_encode(&sha256_bytes_public(canonical_request.as_bytes()));
        let string_to_sign = format!("AWS4-HMAC-SHA256\n{date_time}\n{scope}\n{cr_hash}");

        // Signing key
        let sig_key = {
            let k1 = hmac_sha256(
                format!("AWS4{}", self.secret_access_key).as_bytes(),
                date_only.as_bytes(),
            );
            let k2 = hmac_sha256(&k1, self.region.as_bytes());
            let k3 = hmac_sha256(&k2, b"s3");
            hmac_sha256(&k3, b"aws4_request")
        };
        let signature = hex_encode(&hmac_sha256(&sig_key, string_to_sign.as_bytes()));

        let auth = format!(
            "AWS4-HMAC-SHA256 Credential={}/{},SignedHeaders={},Signature={}",
            self.access_key_id, scope, signed_headers, signature
        );

        let resp = self
            .client
            .put(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/x-ndjson")
            .header("Content-Length", content_len)
            .header("x-amz-content-sha256", content_sha256)
            .header("x-amz-date", date_time)
            .body(body.to_vec())
            .send()
            .map_err(|e| format!("s3 put failed: {e}"))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "s3 returned {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            ))
        }
    }

    fn host(&self) -> String {
        if let Some(ep) = &self.endpoint_override {
            ep.trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_string()
        } else {
            format!("{}.s3.{}.amazonaws.com", self.bucket, self.region)
        }
    }

    fn build_key(&self, events: &[AuditEvent]) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let y = now / 31_536_000 + 1970;
        let day_of_year = (now % 31_536_000) / 86400;
        let month = day_of_year / 30 + 1;
        let day = day_of_year % 30 + 1;
        let seq_from = events.first().map(|e| e.sequence).unwrap_or(0);
        let seq_to = events.last().map(|e| e.sequence).unwrap_or(0);
        let bundle_hash = hex_encode(&sha256_bytes_public(
            events
                .iter()
                .map(|e| e.hash.as_str())
                .collect::<Vec<_>>()
                .join("")
                .as_bytes(),
        ));
        format!(
            "{}/{:04}/{:02}/{:02}/seq-{}-{}-{}.jsonl",
            self.prefix.trim_end_matches('/'),
            y,
            month.min(12),
            day.min(31),
            seq_from,
            seq_to,
            &bundle_hash[..12],
        )
    }
}

impl AuditSink for S3AuditSink {
    fn write_event(&self, event: &AuditEvent) -> Result<(), String> {
        let should_flush = {
            let mut buf = self.buffer.lock().map_err(|e| e.to_string())?;
            buf.push(event.clone());
            buf.len() >= self.batch_size
        };
        if should_flush {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        let batch: Vec<AuditEvent> = {
            let mut buf = self.buffer.lock().map_err(|e| e.to_string())?;
            std::mem::take(&mut *buf)
        };
        if batch.is_empty() {
            return Ok(());
        }
        let key = self.build_key(&batch);
        let body: Vec<u8> = batch
            .iter()
            .map(super::event::AuditEvent::to_json_line)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        if let Err(e) = self.upload(&key, &body) {
            // Re-queue on failure
            let mut buf = self.buffer.lock().map_err(|e2| e2.to_string())?;
            let mut requeued = batch;
            requeued.append(&mut buf);
            *buf = requeued;
            return Err(e);
        }
        Ok(())
    }
}

// ── HMAC-SHA256 (pure Rust, no extra deps) ───────────────────────────────────

/// HMAC-SHA256 implemented over the workspace's own SHA-256 primitive.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let h = sha256_bytes_public(key);
        key_block[..32].copy_from_slice(&h);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Vec::with_capacity(BLOCK + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let inner_hash = sha256_bytes_public(&inner);

    let mut outer = Vec::with_capacity(BLOCK + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha256_bytes_public(&outer)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Format Unix seconds as AWS-style `YYYYMMDDTHHmmssZ`.
fn format_iso8601(secs: u64) -> String {
    let s = secs;
    let ss = s % 60;
    let mm = (s / 60) % 60;
    let hh = (s / 3600) % 24;
    let days = s / 86400;
    // Gregorian approximation (good for 1970–2099)
    let y = (days * 400 + 365) / 146_097 + 1970;
    let leap = |yr: u64| yr % 4 == 0 && (yr % 100 != 0 || yr % 400 == 0);
    let days_in_year = |yr: u64| if leap(yr) { 366u64 } else { 365 };
    let mut rem = days;
    let mut yr = 1970u64;
    while rem >= days_in_year(yr) {
        rem -= days_in_year(yr);
        yr += 1;
    }
    let month_days: [u64; 12] = [
        31,
        if leap(yr) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0usize;
    while mo < 11 && rem >= month_days[mo] {
        rem -= month_days[mo];
        mo += 1;
    }
    let _ = y; // suppress unused warning — yr is the correct year
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        yr,
        mo + 1,
        rem + 1,
        hh,
        mm,
        ss
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AuditEventKind, AuditSeverity};

    #[test]
    fn memory_sink_captures_events() {
        let sink = MemoryAuditSink::new();
        let mut logger = AuditLogger::new();
        logger.add_sink(sink.clone());

        let event = AuditEvent::new("s1", AuditEventKind::SessionStart, "started");
        logger.log(&event);
        logger.log(
            &AuditEvent::new("s1", AuditEventKind::ToolInvocation, "bash pwd")
                .with_severity(AuditSeverity::Warning)
                .with_tool("bash"),
        );

        let events = sink.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, AuditEventKind::SessionStart);
        assert_eq!(events[1].tool_name.as_deref(), Some("bash"));
    }

    // ── HttpAuditSink tests ───────────────────────────────────────────────────

    #[test]
    fn http_sink_buffers_until_batch_size() {
        // Batch size of 3 — first two writes should not trigger a flush attempt.
        // We confirm by checking no network call is made (no server running).
        let sink = HttpAuditSink::new("http://127.0.0.1:1", None, Some(3));
        let event = AuditEvent::new("s1", AuditEventKind::SessionStart, "start");

        // Two events: below batch threshold, should not attempt flush.
        assert!(sink.write_event(&event).is_ok());
        assert!(sink.write_event(&event).is_ok());

        // Inspect buffer length via flush — it will fail (no server) but only
        // after the buffer has been drained into the attempt.
        let err = sink.flush().unwrap_err();
        assert!(
            err.contains("Connection refused") || err.contains("connect") || err.contains("error"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn http_sink_requeues_on_failure() {
        let sink = HttpAuditSink::new("http://127.0.0.1:1", None, Some(100));
        let event = AuditEvent::new("s1", AuditEventKind::ToolInvocation, "run");
        sink.write_event(&event).unwrap();
        sink.write_event(&event).unwrap();

        // First flush fails (no server) — events should be re-queued.
        let _ = sink.flush();

        // Second flush should also fail with re-queued events (not empty).
        let err = sink.flush();
        // If it returned Ok(()) the buffer was empty — that would mean events were lost.
        assert!(
            err.is_err(),
            "events should have been re-queued after first failed flush"
        );
    }

    #[test]
    fn http_sink_custom_headers_chained() {
        let sink = HttpAuditSink::new("http://127.0.0.1:1", Some("tok".to_string()), Some(10))
            .with_header("X-Index", "audit")
            .with_header("X-Env", "test");
        // Verify the sink was constructed without panic.
        let event = AuditEvent::new("s1", AuditEventKind::SessionEnd, "done");
        assert!(sink.write_event(&event).is_ok());
    }

    // ── S3AuditSink tests ─────────────────────────────────────────────────────

    #[test]
    fn s3_sink_key_format_contains_prefix_and_sequences() {
        let sink = S3AuditSink::new(
            "my-bucket",
            "tachy/audit",
            "us-east-1",
            "AKID",
            "SECRET",
            None,
        );
        let mut events: Vec<AuditEvent> = Vec::new();
        for i in 0..3 {
            let mut e = AuditEvent::new("s1", AuditEventKind::ToolInvocation, format!("cmd {i}"));
            e.sequence = i as u64 + 10;
            e.hash = format!("hash{i:040}");
            events.push(e);
        }
        let key = sink.build_key(&events);
        assert!(
            key.starts_with("tachy/audit/"),
            "key should start with prefix: {key}"
        );
        assert!(
            key.contains("seq-10-12-"),
            "key should encode seq range: {key}"
        );
        assert!(key.ends_with(".jsonl"), "key should end with .jsonl: {key}");
    }

    #[test]
    fn s3_sink_with_endpoint_override() {
        let sink = S3AuditSink::new("bucket", "prefix", "us-east-1", "AK", "SK", None)
            .with_endpoint("http://minio:9000");
        let url = format!("{}/some-key", sink.base_url());
        assert!(
            url.starts_with("http://minio:9000/bucket/some-key"),
            "got: {url}"
        );
    }

    #[test]
    fn hmac_sha256_produces_deterministic_output() {
        let a = hmac_sha256(b"key", b"data");
        let b = hmac_sha256(b"key", b"data");
        assert_eq!(a, b);
        let c = hmac_sha256(b"other-key", b"data");
        assert_ne!(a, c);
    }

    #[test]
    fn format_iso8601_round_trip() {
        // 2024-01-15 11:30:45 UTC = 1705318245
        let s = format_iso8601(1_705_318_245);
        assert_eq!(s, "20240115T113045Z");
    }

    #[test]
    fn file_sink_writes_json_lines() {
        let dir = std::env::temp_dir().join(format!(
            "audit-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("audit.jsonl");
        let sink = FileAuditSink::new(&path).expect("should create file sink");

        sink.write_event(&AuditEvent::new("s1", AuditEventKind::SessionStart, "go"))
            .expect("write should succeed");
        sink.flush().expect("flush should succeed");

        let content = fs::read_to_string(&path).expect("should read audit log");
        assert!(content.contains("session_start"));

        fs::remove_dir_all(dir).ok();
    }
}
