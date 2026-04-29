//! Lightweight OpenTelemetry-compatible tracing for Tachy.
//!
//! Instruments three key paths:
//!   - Agent runs (span: tachy.agent.run)
//!   - Tool invocations (span: tachy.tool.invoke)
//!   - LLM calls (span: tachy.llm.generate)
//!
//! Spans are collected in-process and flushed to an OTLP-compatible collector
//! via `POST /v1/traces` (JSON format) when `TACHY_OTLP_ENDPOINT` is set.
//! Without the env var, telemetry is a no-op.
//!
//! This avoids the full opentelemetry-sdk dependency tree while still emitting
//! valid OTLP JSON that Grafana Tempo, Honeycomb, and Datadog can ingest.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single telemetry span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub attributes: BTreeMap<String, serde_json::Value>,
    pub status: SpanStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpanStatus {
    #[default]
    Unset,
    Ok,
    Error,
}

/// A span that is in-progress (not yet ended).
pub struct ActiveSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub start_time_unix_nano: u64,
    pub attributes: BTreeMap<String, serde_json::Value>,
    collector: Arc<Mutex<SpanCollector>>,
}

impl ActiveSpan {
    pub fn set_attr(&mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) {
        self.attributes.insert(key.into(), value.into());
    }

    pub fn finish(self) {
        self.finish_with_status(SpanStatus::Ok);
    }

    pub fn finish_error(self, message: &str) {
        let mut this = self;
        this.set_attr("error.message", message);
        this.finish_with_status(SpanStatus::Error);
    }

    fn finish_with_status(self, status: SpanStatus) {
        let end = now_nano();
        let span = Span {
            trace_id: self.trace_id,
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            name: self.name,
            start_time_unix_nano: self.start_time_unix_nano,
            end_time_unix_nano: end,
            attributes: self.attributes,
            status,
        };
        if let Ok(mut c) = self.collector.lock() {
            c.record(span);
        }
    }
}

// ── Collector ─────────────────────────────────────────────────────────────────

/// In-process span buffer — periodically flushed to the OTLP endpoint.
pub struct SpanCollector {
    spans: Vec<Span>,
    otlp_endpoint: Option<String>,
    service_name: String,
}

impl Default for SpanCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl SpanCollector {
    #[must_use] pub fn new() -> Self {
        Self {
            spans: Vec::new(),
            otlp_endpoint: std::env::var("TACHY_OTLP_ENDPOINT").ok(),
            service_name: std::env::var("TACHY_SERVICE_NAME")
                .unwrap_or_else(|_| "tachy-daemon".to_string()),
        }
    }

    #[must_use] pub fn is_enabled(&self) -> bool {
        self.otlp_endpoint.is_some()
    }

    pub fn record(&mut self, span: Span) {
        self.spans.push(span);
        // Flush every 100 spans to bound memory usage
        if self.spans.len() >= 100 {
            self.flush();
        }
    }

    /// Export buffered spans to the OTLP endpoint (fire-and-forget).
    pub fn flush(&mut self) {
        if self.spans.is_empty() { return; }
        let Some(endpoint) = &self.otlp_endpoint else { self.spans.clear(); return; };

        let url = format!("{endpoint}/v1/traces");
        let payload = build_otlp_payload(&self.spans, &self.service_name);
        self.spans.clear();

        let url_clone = url.clone();
        let payload_clone = payload.clone();
        std::thread::spawn(move || {
            let _ = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .and_then(|c| c.post(&url_clone)
                    .header("Content-Type", "application/json")
                    .body(payload_clone)
                    .send());
        });
    }
}

/// OTLP JSON payload builder (OTLP/HTTP JSON format).
fn build_otlp_payload(spans: &[Span], service_name: &str) -> String {
    let span_data: Vec<serde_json::Value> = spans.iter().map(|s| {
        let attrs: Vec<serde_json::Value> = s.attributes.iter().map(|(k, v)| {
            serde_json::json!({ "key": k, "value": { "stringValue": v.to_string() } })
        }).collect();

        let mut obj = serde_json::json!({
            "traceId": s.trace_id,
            "spanId": s.span_id,
            "name": s.name,
            "startTimeUnixNano": s.start_time_unix_nano.to_string(),
            "endTimeUnixNano": s.end_time_unix_nano.to_string(),
            "attributes": attrs,
            "status": { "code": format!("{:?}", s.status) },
        });

        if let Some(parent) = &s.parent_span_id {
            obj["parentSpanId"] = serde_json::Value::String(parent.clone());
        }

        obj
    }).collect();

    serde_json::json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [{
                    "key": "service.name",
                    "value": { "stringValue": service_name },
                }]
            },
            "scopeSpans": [{
                "scope": { "name": "tachy", "version": "0.1.0" },
                "spans": span_data,
            }]
        }]
    }).to_string()
}

// ── Tracer ────────────────────────────────────────────────────────────────────

/// Global tracer — wraps the shared collector.
#[derive(Clone)]
pub struct Tracer {
    collector: Arc<Mutex<SpanCollector>>,
}

impl Tracer {
    pub fn new(collector: Arc<Mutex<SpanCollector>>) -> Self {
        Self { collector }
    }

    #[must_use] pub fn is_enabled(&self) -> bool {
        self.collector.lock().map(|c| c.is_enabled()).unwrap_or(false)
    }

    /// Start a new root span (no parent).
    pub fn start_span(&self, name: impl Into<String>) -> ActiveSpan {
        self.start_child_span(name, None)
    }

    /// Start a child span under an existing trace.
    pub fn start_child_span(&self, name: impl Into<String>, parent_span_id: Option<String>) -> ActiveSpan {
        ActiveSpan {
            trace_id: random_id(16),
            span_id: random_id(8),
            parent_span_id,
            name: name.into(),
            start_time_unix_nano: now_nano(),
            attributes: BTreeMap::new(),
            collector: Arc::clone(&self.collector),
        }
    }

    pub fn flush(&self) {
        if let Ok(mut c) = self.collector.lock() {
            c.flush();
        }
    }
}

// ── Convenience macros / instrument functions ─────────────────────────────────

/// Record a completed agent run span.
pub fn record_agent_run(
    tracer: &Tracer,
    agent_id: &str,
    model: &str,
    template: &str,
    success: bool,
    iterations: usize,
    tool_invocations: u32,
    duration_ms: u64,
) {
    if !tracer.is_enabled() { return; }
    let mut span = tracer.start_span("tachy.agent.run");
    span.set_attr("agent.id", agent_id);
    span.set_attr("agent.model", model);
    span.set_attr("agent.template", template);
    span.set_attr("agent.iterations", iterations as i64);
    span.set_attr("agent.tool_invocations", i64::from(tool_invocations));
    span.set_attr("agent.duration_ms", duration_ms as i64);
    if success { span.finish(); } else { span.finish_error("agent run failed"); }
}

/// Record a tool invocation span.
pub fn record_tool_invocation(
    tracer: &Tracer,
    tool_name: &str,
    agent_id: &str,
    success: bool,
    duration_ms: u64,
) {
    if !tracer.is_enabled() { return; }
    let mut span = tracer.start_span("tachy.tool.invoke");
    span.set_attr("tool.name", tool_name);
    span.set_attr("agent.id", agent_id);
    span.set_attr("tool.duration_ms", duration_ms as i64);
    if success { span.finish(); } else { span.finish_error("tool invocation failed"); }
}

/// Record an LLM generation span.
pub fn record_llm_call(
    tracer: &Tracer,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    ttft_ms: u32,
    success: bool,
) {
    if !tracer.is_enabled() { return; }
    let mut span = tracer.start_span("tachy.llm.generate");
    span.set_attr("llm.model", model);
    span.set_attr("llm.input_tokens", input_tokens as i64);
    span.set_attr("llm.output_tokens", output_tokens as i64);
    span.set_attr("llm.ttft_ms", i64::from(ttft_ms));
    if success { span.finish(); } else { span.finish_error("LLM call failed"); }
}

/// Record a visual perception event (screenshot).
pub fn record_vision_snapshot(
    tracer: &Tracer,
    agent_id: &str,
    url: &str,
    path: &str,
) {
    if !tracer.is_enabled() { return; }
    let mut span = tracer.start_span("tachy.vision.snapshot");
    span.set_attr("agent.id", agent_id);
    span.set_attr("vision.url", url);
    span.set_attr("vision.path", path);
    span.finish();
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn now_nano() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn random_id(bytes: usize) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let ts = now_nano();
    let pid = u64::from(std::process::id());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{ts}:{pid}:{n}").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}{ts:016x}")
        .chars()
        .take(bytes * 2)
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracer() -> (Tracer, Arc<Mutex<SpanCollector>>) {
        let col = Arc::new(Mutex::new(SpanCollector::new()));
        (Tracer::new(Arc::clone(&col)), col)
    }

    #[test]
    fn span_records_on_finish() {
        let (tracer, col) = make_tracer();
        let span = tracer.start_span("test.span");
        span.finish();
        let c = col.lock().unwrap();
        assert_eq!(c.spans.len(), 1);
        assert_eq!(c.spans[0].name, "test.span");
    }

    #[test]
    fn span_error_sets_status() {
        let (tracer, col) = make_tracer();
        let span = tracer.start_span("error.span");
        span.finish_error("something went wrong");
        let c = col.lock().unwrap();
        assert!(matches!(c.spans[0].status, SpanStatus::Error));
        assert!(c.spans[0].attributes.contains_key("error.message"));
    }

    #[test]
    fn span_attributes_preserved() {
        let (tracer, col) = make_tracer();
        let mut span = tracer.start_span("attr.span");
        span.set_attr("model", "gemma4:26b");
        span.set_attr("iterations", 5i64);
        span.finish();
        let c = col.lock().unwrap();
        assert_eq!(c.spans[0].attributes["model"], "gemma4:26b");
    }

    #[test]
    fn flush_clears_span_buffer() {
        let (tracer, col) = make_tracer();
        // No OTLP endpoint — flush just clears
        let span = tracer.start_span("s");
        span.finish();
        assert_eq!(col.lock().unwrap().spans.len(), 1);
        tracer.flush();
        assert!(col.lock().unwrap().spans.is_empty());
    }

    #[test]
    fn random_ids_unique() {
        let a = random_id(8);
        let b = random_id(8);
        assert_ne!(a, b);
        assert_eq!(a.len(), 16); // bytes * 2 hex chars
    }

    #[test]
    fn record_agent_run_noop_when_disabled() {
        let (tracer, col) = make_tracer();
        // No OTLP endpoint → is_enabled() = false → spans not recorded via helper
        record_agent_run(&tracer, "a1", "gemma4", "chat", true, 3, 5, 1200);
        // Even though is_enabled() is false, the helper returns early — no spans
        let c = col.lock().unwrap();
        assert!(c.spans.is_empty());
    }
}
