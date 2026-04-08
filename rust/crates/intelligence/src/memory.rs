//! Persistent agent memory — survives across sessions.
//!
//! The agent reads `.tachy/memory.md` at the start of every session and can
//! write to it. Stores: user preferences, project context, past decisions,
//! learned patterns. This is what makes the agent feel like it "knows" your
//! project over time.
//!
//! Memory is a simple append-only Markdown file. The agent can:
//! - Read the full memory at session start (injected into system prompt)
//! - Append new memories via the `remember` tool
//! - The file is human-readable and version-controllable

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

const MAX_MEMORY_BYTES: usize = 8000;

/// A memory entry with timestamp and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub timestamp: String,
    pub content: String,
    pub category: MemoryCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Preference,
    ProjectContext,
    Decision,
    Pattern,
    Note,
}

/// Persistent memory manager.
pub struct AgentMemory {
    path: PathBuf,
    entries: Vec<MemoryEntry>,
}

impl AgentMemory {
    /// Load memory from `.tachy/memory.md`.
    #[must_use] pub fn load(tachy_dir: &Path) -> Self {
        let path = tachy_dir.join("memory.md");
        let entries = if path.exists() {
            parse_memory_file(&path)
        } else {
            Vec::new()
        };
        Self { path, entries }
    }

    /// Get memory content formatted for injection into the system prompt.
    /// Truncates to fit within token budget.
    #[must_use] pub fn as_system_context(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        let mut sections = vec!["# Agent Memory (persistent across sessions)".to_string()];
        let mut total_len = 0;

        // Most recent entries first (they're most relevant)
        for entry in self.entries.iter().rev() {
            let line = format!("- [{}] {}", category_label(&entry.category), entry.content);
            if total_len + line.len() > MAX_MEMORY_BYTES {
                break;
            }
            total_len += line.len();
            sections.push(line);
        }

        if sections.len() <= 1 {
            return None;
        }

        Some(sections.join("\n"))
    }

    /// Add a new memory entry and persist to disk.
    pub fn remember(&mut self, content: &str, category: MemoryCategory) -> Result<(), String> {
        let entry = MemoryEntry {
            timestamp: now_timestamp(),
            content: content.to_string(),
            category,
        };

        self.entries.push(entry.clone());
        self.append_to_file(&entry)
    }

    /// Get all entries.
    #[must_use] pub fn entries(&self) -> &[MemoryEntry] {
        &self.entries
    }

    /// Append a single entry to the memory file.
    fn append_to_file(&self, entry: &MemoryEntry) -> Result<(), String> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("failed to open memory file: {e}"))?;

        // Write as markdown
        writeln!(file, "\n## {} — {}", entry.timestamp, category_label(&entry.category))
            .map_err(|e| format!("write failed: {e}"))?;
        writeln!(file, "{}", entry.content)
            .map_err(|e| format!("write failed: {e}"))?;

        Ok(())
    }
}

fn category_label(cat: &MemoryCategory) -> &'static str {
    match cat {
        MemoryCategory::Preference => "preference",
        MemoryCategory::ProjectContext => "project",
        MemoryCategory::Decision => "decision",
        MemoryCategory::Pattern => "pattern",
        MemoryCategory::Note => "note",
    }
}

/// Parse the memory.md file into entries.
fn parse_memory_file(path: &Path) -> Vec<MemoryEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut current_timestamp = String::new();
    let mut current_category = MemoryCategory::Note;
    let mut current_content = String::new();

    for line in content.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            // Save previous entry
            if !current_content.trim().is_empty() {
                entries.push(MemoryEntry {
                    timestamp: current_timestamp.clone(),
                    content: current_content.trim().to_string(),
                    category: current_category.clone(),
                });
            }
            // Parse header: "## 2026-04-03T12:00:00 — preference"
            if let Some((ts, cat)) = header.split_once(" — ") {
                current_timestamp = ts.trim().to_string();
                current_category = match cat.trim() {
                    "preference" => MemoryCategory::Preference,
                    "project" => MemoryCategory::ProjectContext,
                    "decision" => MemoryCategory::Decision,
                    "pattern" => MemoryCategory::Pattern,
                    _ => MemoryCategory::Note,
                };
            } else {
                current_timestamp = header.to_string();
                current_category = MemoryCategory::Note;
            }
            current_content.clear();
        } else if !line.starts_with("# ") {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            current_content.push_str(line);
        }
    }

    // Save last entry
    if !current_content.trim().is_empty() {
        entries.push(MemoryEntry {
            timestamp: current_timestamp,
            content: current_content.trim().to_string(),
            category: current_category,
        });
    }

    entries
}

fn now_timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

/// Execute the "remember" tool — called by the LLM to store a memory.
pub fn execute_remember(input: &serde_json::Value, tachy_dir: &Path) -> Result<String, String> {
    let content = input.get("content").and_then(|v| v.as_str())
        .ok_or("'content' parameter required")?;
    let category = input.get("category").and_then(|v| v.as_str()).unwrap_or("note");
    let cat = match category {
        "preference" => MemoryCategory::Preference,
        "project" => MemoryCategory::ProjectContext,
        "decision" => MemoryCategory::Decision,
        "pattern" => MemoryCategory::Pattern,
        _ => MemoryCategory::Note,
    };

    let mut memory = AgentMemory::load(tachy_dir);
    memory.remember(content, cat)?;
    Ok(format!("Remembered: {content}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_memory_returns_none() {
        let mem = AgentMemory { path: PathBuf::from("/tmp/nonexistent"), entries: Vec::new() };
        assert!(mem.as_system_context().is_none());
    }

    #[test]
    fn memory_formats_as_context() {
        let mem = AgentMemory {
            path: PathBuf::from("/tmp/test"),
            entries: vec![
                MemoryEntry { timestamp: "1".to_string(), content: "User prefers Rust".to_string(), category: MemoryCategory::Preference },
                MemoryEntry { timestamp: "2".to_string(), content: "Project uses PostgreSQL".to_string(), category: MemoryCategory::ProjectContext },
            ],
        };
        let ctx = mem.as_system_context().unwrap();
        assert!(ctx.contains("Agent Memory"));
        assert!(ctx.contains("User prefers Rust"));
        assert!(ctx.contains("PostgreSQL"));
    }

    #[test]
    fn parses_memory_file() {
        let dir = std::env::temp_dir().join(format!("tachy-mem-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.md");
        std::fs::write(&path, "# Agent Memory\n\n## 100s — preference\nUser likes concise answers\n\n## 200s — project\nThis is a Rust project\n").unwrap();

        let entries = parse_memory_file(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].category, MemoryCategory::Preference);
        assert!(entries[0].content.contains("concise"));
        assert_eq!(entries[1].category, MemoryCategory::ProjectContext);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn remember_appends_to_file() {
        let dir = std::env::temp_dir().join(format!("tachy-mem-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut mem = AgentMemory::load(&dir);
        mem.remember("Test memory", MemoryCategory::Note).unwrap();
        mem.remember("Another memory", MemoryCategory::Decision).unwrap();

        // Reload and verify
        let mem2 = AgentMemory::load(&dir);
        assert_eq!(mem2.entries().len(), 2);
        assert!(mem2.entries()[0].content.contains("Test memory"));

        std::fs::remove_dir_all(dir).ok();
    }
}
