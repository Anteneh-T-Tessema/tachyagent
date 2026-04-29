//! Opt-in telemetry — anonymous usage analytics for improving Tachy.
//!
//! Disabled by default. Enable with `TACHY_TELEMETRY=1` or in config.
//! Data collected: model name, tool success rates, iteration counts.
//! NO prompts, NO file contents, NO code, NO PII.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Telemetry configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub enabled: bool,
    /// Anonymous machine ID (SHA-256 of hostname).
    pub machine_id: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("TACHY_TELEMETRY")
                .map(|v| v == "1")
                .unwrap_or(false),
            machine_id: String::new(),
        }
    }
}

/// A single telemetry event (anonymous, no PII).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub event_type: String,
    pub model: String,
    pub iterations: usize,
    pub tool_invocations: u32,
    pub success: bool,
    pub duration_ms: u64,
    pub timestamp: u64,
}

/// Telemetry collector — buffers events and flushes to disk.
pub struct TelemetryCollector {
    config: TelemetryConfig,
    events: Vec<TelemetryEvent>,
    flush_path: PathBuf,
}

impl TelemetryCollector {
    #[must_use]
    pub fn new(config: TelemetryConfig, tachy_dir: &Path) -> Self {
        Self {
            config,
            events: Vec::new(),
            flush_path: tachy_dir.join("telemetry.jsonl"),
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Record an agent run event.
    pub fn record_agent_run(
        &mut self,
        model: &str,
        iterations: usize,
        tool_invocations: u32,
        success: bool,
        duration_ms: u64,
    ) {
        if !self.config.enabled {
            return;
        }
        self.events.push(TelemetryEvent {
            event_type: "agent_run".to_string(),
            model: model.to_string(),
            iterations,
            tool_invocations,
            success,
            duration_ms,
            timestamp: now_epoch(),
        });

        // Auto-flush every 10 events
        if self.events.len() >= 10 {
            self.flush();
        }
    }

    /// Flush buffered events to disk.
    pub fn flush(&mut self) {
        if self.events.is_empty() {
            return;
        }
        if let Some(parent) = self.flush_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.flush_path);

        if let Ok(ref mut f) = file {
            use std::io::Write;
            for event in &self.events {
                if let Ok(json) = serde_json::to_string(event) {
                    let _ = writeln!(f, "{json}");
                }
            }
        }
        self.events.clear();
    }

    /// Get a summary of collected telemetry.
    #[must_use]
    pub fn summary(&self) -> TelemetrySummary {
        let total = self.events.len();
        let successes = self.events.iter().filter(|e| e.success).count();
        let mut by_model: BTreeMap<String, usize> = BTreeMap::new();
        for event in &self.events {
            *by_model.entry(event.model.clone()).or_insert(0) += 1;
        }
        TelemetrySummary {
            total_events: total,
            success_rate: if total > 0 {
                successes as f64 / total as f64
            } else {
                0.0
            },
            by_model,
        }
    }
}

impl Drop for TelemetryCollector {
    fn drop(&mut self) {
        self.flush();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySummary {
    pub total_events: usize,
    pub success_rate: f64,
    pub by_model: BTreeMap<String, usize>,
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_disabled_by_default() {
        let config = TelemetryConfig::default();
        assert!(!config.enabled);
    }

    #[test]
    fn telemetry_records_when_enabled() {
        let dir = std::env::temp_dir().join(format!("tachy-tel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let config = TelemetryConfig {
            enabled: true,
            machine_id: "test".to_string(),
        };
        let mut collector = TelemetryCollector::new(config, &dir);

        collector.record_agent_run("gemma4:26b", 3, 5, true, 1200);
        collector.record_agent_run("qwen3:8b", 1, 0, false, 500);

        let summary = collector.summary();
        assert_eq!(summary.total_events, 2);
        assert!((summary.success_rate - 0.5).abs() < f64::EPSILON);
        assert_eq!(summary.by_model.get("gemma4:26b"), Some(&1));

        collector.flush();
        assert!(dir.join("telemetry.jsonl").exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn telemetry_skips_when_disabled() {
        let dir = std::env::temp_dir().join(format!("tachy-tel-off-{}", std::process::id()));
        let config = TelemetryConfig::default(); // disabled
        let mut collector = TelemetryCollector::new(config, &dir);

        collector.record_agent_run("model", 1, 0, true, 100);
        assert_eq!(collector.summary().total_events, 0);
    }
}
