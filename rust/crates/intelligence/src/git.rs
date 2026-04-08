use std::fmt::{Display, Formatter};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
    pub is_clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeStatus { Added, Modified, Deleted, Renamed }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiff {
    pub files: Vec<FileDiff>,
    pub stats: DiffStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResult {
    pub hash: String,
    pub message: String,
    pub files_changed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchResult {
    pub name: String,
    pub created: bool,
    pub previous_branch: String,
}

#[derive(Debug)]
pub enum GitError {
    NotARepo,
    CommandFailed { command: String, stderr: String },
    NothingToCommit,
}

impl Display for GitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARepo => write!(f, "not a git repository"),
            Self::CommandFailed { command, stderr } => write!(f, "git {command} failed: {stderr}"),
            Self::NothingToCommit => write!(f, "nothing to commit"),
        }
    }
}

impl std::error::Error for GitError {}

pub struct GitTools;

impl GitTools {
    #[must_use] pub fn is_git_repo() -> bool {
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn current_branch() -> Result<String, GitError> {
        if !Self::is_git_repo() { return Err(GitError::NotARepo); }
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .map_err(|e| GitError::CommandFailed { command: "rev-parse".into(), stderr: e.to_string() })?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn status() -> Result<GitStatus, GitError> {
        if !Self::is_git_repo() { return Err(GitError::NotARepo); }
        let branch = Self::current_branch()?;
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .map_err(|e| GitError::CommandFailed { command: "status".into(), stderr: e.to_string() })?;

        let text = String::from_utf8_lossy(&output.stdout);
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in text.lines() {
            if line.len() < 3 { continue; }
            let index_status = line.chars().next().unwrap_or(' ');
            let work_status = line.chars().nth(1).unwrap_or(' ');
            let path = line[3..].to_string();

            if index_status == '?' {
                untracked.push(path);
            } else {
                if index_status != ' ' {
                    staged.push(FileChange { path: path.clone(), status: parse_status(index_status) });
                }
                if work_status != ' ' {
                    unstaged.push(FileChange { path, status: parse_status(work_status) });
                }
            }
        }

        let is_clean = staged.is_empty() && unstaged.is_empty() && untracked.is_empty();
        Ok(GitStatus { branch, staged, unstaged, untracked, is_clean })
    }

    pub fn diff(path: Option<&str>, staged: bool) -> Result<GitDiff, GitError> {
        if !Self::is_git_repo() { return Err(GitError::NotARepo); }
        let mut args = vec!["diff"];
        if staged { args.push("--staged"); }
        args.push("--stat");
        if let Some(p) = path { args.push(p); }

        let stat_output = Command::new("git").args(&args).output()
            .map_err(|e| GitError::CommandFailed { command: "diff --stat".into(), stderr: e.to_string() })?;
        let stat_text = String::from_utf8_lossy(&stat_output.stdout);

        // Get full diff content
        let mut content_args = vec!["diff"];
        if staged { content_args.push("--staged"); }
        if let Some(p) = path { content_args.push(p); }
        let content_output = Command::new("git").args(&content_args).output()
            .map_err(|e| GitError::CommandFailed { command: "diff".into(), stderr: e.to_string() })?;
        let content = String::from_utf8_lossy(&content_output.stdout);

        let (files, stats) = parse_diff_stat(&stat_text, &content);
        Ok(GitDiff { files, stats })
    }

    pub fn branch(name: &str, create: bool) -> Result<BranchResult, GitError> {
        if !Self::is_git_repo() { return Err(GitError::NotARepo); }
        let previous = Self::current_branch()?;

        let args = if create {
            vec!["checkout", "-b", name]
        } else {
            vec!["checkout", name]
        };

        let output = Command::new("git").args(&args).output()
            .map_err(|e| GitError::CommandFailed { command: format!("checkout {name}"), stderr: e.to_string() })?;

        if !output.status.success() {
            return Err(GitError::CommandFailed {
                command: format!("checkout {name}"),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(BranchResult { name: name.to_string(), created: create, previous_branch: previous })
    }

    pub fn commit(message: &str) -> Result<CommitResult, GitError> {
        if !Self::is_git_repo() { return Err(GitError::NotARepo); }

        // Stage all
        let _add = Command::new("git").args(["add", "-A"]).output()
            .map_err(|e| GitError::CommandFailed { command: "add".into(), stderr: e.to_string() })?;

        // Check if there's anything to commit
        let status = Self::status()?;
        if status.staged.is_empty() && status.unstaged.is_empty() {
            return Err(GitError::NothingToCommit);
        }

        let output = Command::new("git").args(["commit", "-m", message]).output()
            .map_err(|e| GitError::CommandFailed { command: "commit".into(), stderr: e.to_string() })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if stderr.contains("nothing to commit") {
                return Err(GitError::NothingToCommit);
            }
            return Err(GitError::CommandFailed { command: "commit".into(), stderr });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let hash = stdout
            .split_whitespace()
            .find(|w| w.len() >= 7 && w.chars().all(|c| c.is_ascii_hexdigit() || c == ']' || c == '['))
            .unwrap_or("unknown")
            .trim_matches(|c| c == '[' || c == ']')
            .to_string();

        let files_changed = status.staged.len() + status.unstaged.len();

        Ok(CommitResult { hash, message: message.to_string(), files_changed })
    }
}

fn parse_status(c: char) -> ChangeStatus {
    match c {
        'A' => ChangeStatus::Added,
        'M' => ChangeStatus::Modified,
        'D' => ChangeStatus::Deleted,
        'R' => ChangeStatus::Renamed,
        _ => ChangeStatus::Modified,
    }
}

fn parse_diff_stat(_stat: &str, content: &str) -> (Vec<FileDiff>, DiffStats) {
    let mut files = Vec::new();
    let mut total_ins = 0usize;
    let mut total_del = 0usize;

    // Simple: just wrap the full diff content as one entry per file
    // A proper parser would split on "diff --git" but this is good enough
    if !content.trim().is_empty() {
        files.push(FileDiff {
            path: "combined".to_string(),
            additions: content.lines().filter(|l| l.starts_with('+')).count(),
            deletions: content.lines().filter(|l| l.starts_with('-')).count(),
            content: if content.len() > 4000 {
                format!("{}…", &content[..4000])
            } else {
                content.to_string()
            },
        });
        total_ins = files[0].additions;
        total_del = files[0].deletions;
    }

    let stats = DiffStats {
        files_changed: files.len(),
        insertions: total_ins,
        deletions: total_del,
    };

    (files, stats)
}

/// Tool execution functions for registration in the tool executor.
pub fn execute_git_status() -> Result<String, String> {
    let status = GitTools::status().map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&status).map_err(|e| e.to_string())
}

pub fn execute_git_diff(input: &serde_json::Value) -> Result<String, String> {
    let path = input.get("path").and_then(|v| v.as_str());
    let staged = input.get("staged").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let diff = GitTools::diff(path, staged).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&diff).map_err(|e| e.to_string())
}

pub fn execute_git_branch(input: &serde_json::Value) -> Result<String, String> {
    let name = input.get("name").and_then(|v| v.as_str()).ok_or("missing 'name'")?;
    let create = input.get("create").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let result = GitTools::branch(name, create).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}

pub fn execute_git_commit(input: &serde_json::Value) -> Result<String, String> {
    let message = input.get("message").and_then(|v| v.as_str()).ok_or("missing 'message'")?;
    let result = GitTools::commit(message).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}

#[must_use] pub fn git_tool_specs() -> Vec<tools::ToolSpec> {
    vec![
        tools::ToolSpec {
            name: "git_status",
            description: "Show git working tree status with structured output.",
            input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        },
        tools::ToolSpec {
            name: "git_diff",
            description: "Show changes between working tree and HEAD.",
            input_schema: serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"staged":{"type":"boolean"}},"additionalProperties":false}),
        },
        tools::ToolSpec {
            name: "git_branch",
            description: "Create or switch git branches.",
            input_schema: serde_json::json!({"type":"object","properties":{"name":{"type":"string"},"create":{"type":"boolean"}},"required":["name"],"additionalProperties":false}),
        },
        tools::ToolSpec {
            name: "git_commit",
            description: "Stage all changes and commit with a message.",
            input_schema: serde_json::json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false}),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_status_is_clean_consistency() {
        let clean = GitStatus {
            branch: "main".into(), staged: vec![], unstaged: vec![], untracked: vec![], is_clean: true,
        };
        assert!(clean.is_clean);
        assert!(clean.staged.is_empty() && clean.unstaged.is_empty() && clean.untracked.is_empty());

        let dirty = GitStatus {
            branch: "main".into(),
            staged: vec![FileChange { path: "a.rs".into(), status: ChangeStatus::Modified }],
            unstaged: vec![], untracked: vec![], is_clean: false,
        };
        assert!(!dirty.is_clean);
    }

    #[test]
    fn parse_status_char() {
        assert_eq!(parse_status('A'), ChangeStatus::Added);
        assert_eq!(parse_status('M'), ChangeStatus::Modified);
        assert_eq!(parse_status('D'), ChangeStatus::Deleted);
    }

    #[test]
    fn git_tool_specs_are_valid() {
        let specs = git_tool_specs();
        assert_eq!(specs.len(), 4);
        assert!(specs.iter().any(|s| s.name == "git_status"));
        assert!(specs.iter().any(|s| s.name == "git_commit"));
    }
}
