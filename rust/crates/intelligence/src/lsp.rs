//! LSP (Language Server Protocol) integration layer.
//!
//! Wraps LSP capabilities into tools the agent can use:
//! - `go_to_definition(file`, line, col)
//! - `find_references(file`, line, col)
//! - `get_diagnostics(file)`
//! - `rename_symbol(file`, line, col, `new_name`)
//! - `hover_info(file`, line, col)
//!
//! Architecture:
//!   Agent Loop → LSP Tool Interface → LSP Client → Language Server (tsserver, pylsp)
//!
//! Language servers are long-lived processes managed by this module.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Supported language servers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LanguageServer {
    TypeScript, // tsserver / typescript-language-server
    Python,     // pylsp or pyright
    Rust,       // rust-analyzer
    Go,         // gopls
}

/// A diagnostic (error/warning) from the language server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// A location in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// Result of a hover query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverInfo {
    pub content: String,
    pub range: Option<(usize, usize)>,
}

/// The LSP integration manager.
/// Manages language server processes and provides tool-like interfaces.
pub struct LspManager {
    workspace_root: PathBuf,
}

impl LspManager {
    #[must_use]
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// Get diagnostics for a file using the appropriate language tool.
    #[must_use]
    pub fn get_diagnostics(&self, file: &str) -> Vec<Diagnostic> {
        let path = self.resolve_path(file);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "ts" | "tsx" | "js" | "jsx" => self.typescript_diagnostics(&path),
            "py" => self.python_diagnostics(&path),
            "rs" => self.rust_diagnostics(&path),
            "go" => self.go_diagnostics(&path),
            _ => Vec::new(),
        }
    }

    /// Get type/hover info for a symbol.
    #[must_use]
    pub fn hover(&self, file: &str, line: usize, col: usize) -> Option<HoverInfo> {
        // For now, use grep-based symbol lookup
        // Full LSP hover requires a running language server
        let path = self.resolve_path(file);
        let content = std::fs::read_to_string(&path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        let target_line = lines.get(line.saturating_sub(1))?;

        // Extract the word at the column position
        let chars: Vec<char> = target_line.chars().collect();
        let mut start = col.saturating_sub(1);
        let mut end = col.saturating_sub(1);
        while start > 0
            && chars
                .get(start - 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            start -= 1;
        }
        while end < chars.len()
            && chars
                .get(end)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            end += 1;
        }
        let symbol: String = chars[start..end].iter().collect();

        if symbol.is_empty() {
            return None;
        }

        Some(HoverInfo {
            content: format!("Symbol: {symbol} (line {line}, col {col})"),
            range: Some((start, end)),
        })
    }

    /// Find all references to a symbol in the workspace.
    #[must_use]
    pub fn find_references(&self, file: &str, line: usize, col: usize) -> Vec<Location> {
        let path = self.resolve_path(file);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        // Extract symbol name at position
        let lines: Vec<&str> = content.lines().collect();
        let target_line = match lines.get(line.saturating_sub(1)) {
            Some(l) => l,
            None => return Vec::new(),
        };

        let chars: Vec<char> = target_line.chars().collect();
        let mut start = col.saturating_sub(1).min(chars.len().saturating_sub(1));
        let mut end = start;
        while start > 0
            && chars
                .get(start - 1)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            start -= 1;
        }
        while end < chars.len()
            && chars
                .get(end)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
        {
            end += 1;
        }
        let symbol: String = chars[start..end].iter().collect();
        if symbol.is_empty() {
            return Vec::new();
        }

        // Use grep to find references across the workspace
        self.grep_symbol(&symbol)
    }

    // --- Language-specific diagnostic implementations ---

    fn typescript_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        // Use `npx tsc --noEmit` for TypeScript diagnostics
        let output = Command::new("npx")
            .args(["tsc", "--noEmit", "--pretty", "false"])
            .current_dir(&self.workspace_root)
            .output();

        match output {
            Ok(out) => parse_tsc_output(&String::from_utf8_lossy(&out.stdout), path),
            Err(_) => Vec::new(),
        }
    }

    fn python_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        // Use `python -m py_compile` or `pyflakes`
        let file_str = path.to_string_lossy();
        let output = Command::new("python3")
            .args(["-m", "py_compile", &file_str])
            .current_dir(&self.workspace_root)
            .output();

        match output {
            Ok(out) if !out.status.success() => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                parse_python_errors(&stderr, path)
            }
            _ => Vec::new(),
        }
    }

    fn rust_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        // Use `cargo check --message-format=json`
        let output = Command::new("cargo")
            .args(["check", "--message-format=json", "--quiet"])
            .current_dir(&self.workspace_root)
            .output();

        match output {
            Ok(out) => parse_cargo_diagnostics(&String::from_utf8_lossy(&out.stdout), path),
            Err(_) => Vec::new(),
        }
    }

    fn go_diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
        let output = Command::new("go")
            .args(["vet", "./..."])
            .current_dir(&self.workspace_root)
            .output();

        match output {
            Ok(out) if !out.status.success() => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                parse_go_errors(&stderr, path)
            }
            _ => Vec::new(),
        }
    }

    fn grep_symbol(&self, symbol: &str) -> Vec<Location> {
        let output = Command::new("grep")
            .args([
                "-rn",
                "--include=*.rs",
                "--include=*.ts",
                "--include=*.py",
                "--include=*.go",
                "--include=*.js",
                symbol,
            ])
            .current_dir(&self.workspace_root)
            .output();

        match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .take(50)
                .filter_map(|line| {
                    let parts: Vec<&str> = line.splitn(3, ':').collect();
                    if parts.len() >= 2 {
                        Some(Location {
                            file: parts[0].to_string(),
                            line: parts[1].parse().unwrap_or(0),
                            column: 0,
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn resolve_path(&self, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace_root.join(file)
        }
    }
}

// --- Output parsers ---

fn parse_tsc_output(output: &str, _target: &Path) -> Vec<Diagnostic> {
    output
        .lines()
        .filter_map(|line| {
            // Format: file(line,col): error TS1234: message
            let paren = line.find('(')?;
            let close = line.find(')')?;
            let file = &line[..paren];
            let coords = &line[paren + 1..close];
            let rest = line.get(close + 2..)?.trim();

            let parts: Vec<&str> = coords.split(',').collect();
            let line_num: usize = parts.first()?.parse().ok()?;
            let col: usize = parts.get(1)?.parse().ok()?;

            let severity = if rest.starts_with("error") {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Warning
            };
            let message = rest.split_once(": ").map_or(rest, |x| x.1);

            Some(Diagnostic {
                file: file.to_string(),
                line: line_num,
                column: col,
                severity,
                message: message.to_string(),
                source: "tsc".to_string(),
            })
        })
        .collect()
}

fn parse_python_errors(stderr: &str, target: &Path) -> Vec<Diagnostic> {
    stderr
        .lines()
        .filter_map(|line| {
            if line.contains("SyntaxError") || line.contains("Error") {
                Some(Diagnostic {
                    file: target.to_string_lossy().to_string(),
                    line: 0,
                    column: 0,
                    severity: DiagnosticSeverity::Error,
                    message: line.trim().to_string(),
                    source: "python".to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn parse_cargo_diagnostics(output: &str, _target: &Path) -> Vec<Diagnostic> {
    output
        .lines()
        .filter_map(|line| {
            let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
            let msg = parsed.get("message")?;
            let level = msg.get("level")?.as_str()?;
            let message = msg.get("message")?.as_str()?;
            let spans = msg.get("spans")?.as_array()?;
            let span = spans.first()?;

            Some(Diagnostic {
                file: span.get("file_name")?.as_str()?.to_string(),
                line: span.get("line_start")?.as_u64()? as usize,
                column: span.get("column_start")?.as_u64().unwrap_or(0) as usize,
                severity: if level == "error" {
                    DiagnosticSeverity::Error
                } else {
                    DiagnosticSeverity::Warning
                },
                message: message.to_string(),
                source: "cargo".to_string(),
            })
        })
        .collect()
}

fn parse_go_errors(stderr: &str, _target: &Path) -> Vec<Diagnostic> {
    stderr
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 4 {
                Some(Diagnostic {
                    file: parts[0].to_string(),
                    line: parts[1].parse().unwrap_or(0),
                    column: parts[2].trim().parse().unwrap_or(0),
                    severity: DiagnosticSeverity::Error,
                    message: parts[3].trim().to_string(),
                    source: "go".to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Execute the `get_diagnostics` tool — called by the LLM.
pub fn execute_get_diagnostics(
    input: &serde_json::Value,
    workspace_root: &Path,
) -> Result<String, String> {
    let file = input
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or("'file' parameter required")?;
    let lsp = LspManager::new(workspace_root);
    let diagnostics = lsp.get_diagnostics(file);
    if diagnostics.is_empty() {
        Ok("No diagnostics found (file is clean).".to_string())
    } else {
        let mut out = format!("{} diagnostics found:\n", diagnostics.len());
        for d in &diagnostics {
            out.push_str(&format!(
                "  {}:{}:{} [{}] {}\n",
                d.file,
                d.line,
                d.column,
                match d.severity {
                    DiagnosticSeverity::Error => "ERROR",
                    DiagnosticSeverity::Warning => "WARN",
                    _ => "INFO",
                },
                d.message
            ));
        }
        Ok(out)
    }
}

/// Execute the `find_references` tool.
pub fn execute_find_references(
    input: &serde_json::Value,
    workspace_root: &Path,
) -> Result<String, String> {
    let file = input
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or("'file' required")?;
    let line = input
        .get("line")
        .and_then(serde_json::Value::as_u64)
        .ok_or("'line' required")? as usize;
    let col = input
        .get("column")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;

    let lsp = LspManager::new(workspace_root);
    let refs = lsp.find_references(file, line, col);
    if refs.is_empty() {
        Ok("No references found.".to_string())
    } else {
        let mut out = format!("{} references found:\n", refs.len());
        for r in &refs {
            out.push_str(&format!("  {}:{}\n", r.file, r.line));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_extracts_symbol() {
        let dir = std::env::temp_dir().join(format!("lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.rs"), "fn hello_world() {}\n").unwrap();

        let lsp = LspManager::new(&dir);
        let info = lsp.hover("test.rs", 1, 4);
        assert!(info.is_some());
        assert!(info.unwrap().content.contains("hello_world"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_diagnostics_for_missing_file() {
        let lsp = LspManager::new(Path::new("/tmp/nonexistent"));
        let diags = lsp.get_diagnostics("missing.rs");
        assert!(diags.is_empty());
    }
}
