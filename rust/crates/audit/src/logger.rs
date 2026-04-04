use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::event::AuditEvent;

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
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
}

impl AuditLogger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sinks: Vec::new(),
            sequence: Mutex::new(0),
            last_hash: Mutex::new(String::new()),
        }
    }

    /// Create a logger that continues the hash chain from an existing audit file.
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
        let mut seq = self.sequence.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = self.last_hash.lock().unwrap_or_else(|e| e.into_inner());

        *seq += 1;
        let mut signed = event.clone();
        signed.sign(*seq, &last);
        *last = signed.hash.clone();

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

    /// Get the current sequence number.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        *self.sequence.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Get the last hash in the chain.
    #[must_use]
    pub fn last_hash(&self) -> String {
        self.last_hash.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
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
