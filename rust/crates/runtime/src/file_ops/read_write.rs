//! Read, write, and edit-file operations with diff previews.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diff::UnifiedDiff;

// ── Shared path helpers (used by search and directory modules too) ────────────

/// Resolve `path` to an absolute, canonical path. Fails if the path does not exist.
pub(super) fn normalize_path(path: &str) -> io::Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()?.join(path)
    };
    candidate.canonicalize()
}

/// Resolve `path` to an absolute path; if the file does not exist yet, canonicalize
/// the parent directory and append the filename.
pub(super) fn normalize_path_allow_missing(path: &str) -> io::Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()?.join(path)
    };

    if let Ok(canonical) = candidate.canonicalize() {
        return Ok(canonical);
    }

    if let Some(parent) = candidate.parent() {
        let canonical_parent = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if let Some(name) = candidate.file_name() {
            return Ok(canonical_parent.join(name));
        }
    }

    Ok(candidate)
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextFilePayload {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "numLines")]
    pub num_lines: usize,
    #[serde(rename = "startLine")]
    pub start_line: usize,
    #[serde(rename = "totalLines")]
    pub total_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    pub file: TextFilePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredPatchHunk {
    #[serde(rename = "oldStart")]
    pub old_start: usize,
    #[serde(rename = "oldLines")]
    pub old_lines: usize,
    #[serde(rename = "newStart")]
    pub new_start: usize,
    #[serde(rename = "newLines")]
    pub new_lines: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "originalFile")]
    pub original_file: Option<String>,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditFileOutput {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "oldString")]
    pub old_string: String,
    #[serde(rename = "newString")]
    pub new_string: String,
    #[serde(rename = "originalFile")]
    pub original_file: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "userModified")]
    pub user_modified: bool,
    #[serde(rename = "replaceAll")]
    pub replace_all: bool,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

/// Result of a diff preview before writing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffPreview {
    /// The file path being modified.
    pub file_path: String,
    /// Unified diff text (compatible with `git apply`).
    pub diff_text: String,
    /// Colored diff text for terminal display.
    pub diff_colored: String,
    /// Summary line (e.g. "file.rs: +5 -3").
    pub summary: String,
    /// Number of additions.
    pub additions: usize,
    /// Number of deletions.
    pub deletions: usize,
    /// Whether this is a new file creation.
    pub is_new_file: bool,
}

// ── Functions ─────────────────────────────────────────────────────────────────

pub fn read_file(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> io::Result<ReadFileOutput> {
    let absolute_path = normalize_path(path)?;
    let content = fs::read_to_string(&absolute_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let start_index = offset.unwrap_or(0).min(lines.len());
    let end_index = limit.map_or(lines.len(), |limit| {
        start_index.saturating_add(limit).min(lines.len())
    });
    let selected = lines[start_index..end_index].join("\n");

    Ok(ReadFileOutput {
        kind: String::from("text"),
        file: TextFilePayload {
            file_path: absolute_path.to_string_lossy().into_owned(),
            content: selected,
            num_lines: end_index.saturating_sub(start_index),
            start_line: start_index.saturating_add(1),
            total_lines: lines.len(),
        },
    })
}

/// Generate a diff preview for `write_file` WITHOUT writing to disk.
pub fn preview_write_file(path: &str, content: &str) -> io::Result<DiffPreview> {
    let absolute_path = normalize_path_allow_missing(path)?;
    let old_content = fs::read_to_string(&absolute_path).unwrap_or_default();
    let is_new = !absolute_path.exists();
    let display_path = absolute_path.to_string_lossy().into_owned();
    let diff = UnifiedDiff::compute(&display_path, &old_content, content);
    Ok(DiffPreview {
        file_path: display_path,
        diff_text: diff.render(),
        diff_colored: diff.render_colored(),
        summary: diff.summary(),
        additions: diff.additions,
        deletions: diff.deletions,
        is_new_file: is_new,
    })
}

/// Generate a diff preview for `edit_file` WITHOUT writing to disk.
pub fn preview_edit_file(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> io::Result<DiffPreview> {
    let absolute_path = normalize_path(path)?;
    let original = fs::read_to_string(&absolute_path)?;
    if old_string == new_string {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_string and new_string must differ",
        ));
    }
    let effective_old = if original.contains(old_string) {
        old_string.to_string()
    } else {
        match fuzzy_find_match(&original, old_string) {
            Some(matched) => matched,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "old_string not found in file",
                ));
            }
        }
    };
    let updated = if replace_all {
        original.replace(&effective_old, new_string)
    } else {
        original.replacen(&effective_old, new_string, 1)
    };
    let display_path = absolute_path.to_string_lossy().into_owned();
    let diff = UnifiedDiff::compute(&display_path, &original, &updated);
    Ok(DiffPreview {
        file_path: display_path,
        diff_text: diff.render(),
        diff_colored: diff.render_colored(),
        summary: diff.summary(),
        additions: diff.additions,
        deletions: diff.deletions,
        is_new_file: false,
    })
}

pub fn write_file(path: &str, content: &str) -> io::Result<(WriteFileOutput, DiffPreview)> {
    let absolute_path = normalize_path_allow_missing(path)?;
    let original_file = fs::read_to_string(&absolute_path).ok();
    let is_new = original_file.is_none();
    let display_path = absolute_path.to_string_lossy().into_owned();

    // Compute diff BEFORE writing
    let diff = UnifiedDiff::compute(
        &display_path,
        original_file.as_deref().unwrap_or(""),
        content,
    );
    let preview = DiffPreview {
        file_path: display_path.clone(),
        diff_text: diff.render(),
        diff_colored: diff.render_colored(),
        summary: diff.summary(),
        additions: diff.additions,
        deletions: diff.deletions,
        is_new_file: is_new,
    };

    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&absolute_path, content)?;

    let output = WriteFileOutput {
        kind: if original_file.is_some() {
            String::from("update")
        } else {
            String::from("create")
        },
        file_path: display_path,
        content: content.to_owned(),
        structured_patch: make_patch(original_file.as_deref().unwrap_or(""), content),
        original_file,
        git_diff: None,
    };
    Ok((output, preview))
}

pub fn edit_file(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> io::Result<(EditFileOutput, DiffPreview)> {
    let absolute_path = normalize_path(path)?;
    let original_file = fs::read_to_string(&absolute_path)?;
    if old_string == new_string {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_string and new_string must differ",
        ));
    }

    // Try exact match first
    let (effective_old, _fuzzy_matched) = if original_file.contains(old_string) {
        (old_string.to_string(), false)
    } else {
        // Fuzzy match: try normalizing whitespace (tabs vs spaces, trailing whitespace)
        if let Some(matched) = fuzzy_find_match(&original_file, old_string) {
            (matched, true)
        } else {
            // Provide a helpful error with nearby context
            let hint = find_closest_match(&original_file, old_string);
            let msg = if let Some(h) = hint {
                format!(
                    "old_string not found in file. Did you mean:\n{}",
                    h.chars().take(200).collect::<String>()
                )
            } else {
                "old_string not found in file".to_string()
            };
            return Err(io::Error::new(io::ErrorKind::NotFound, msg));
        }
    };

    let updated = if replace_all {
        original_file.replace(&effective_old, new_string)
    } else {
        original_file.replacen(&effective_old, new_string, 1)
    };

    // Compute diff BEFORE writing
    let display_path = absolute_path.to_string_lossy().into_owned();
    let diff = UnifiedDiff::compute(&display_path, &original_file, &updated);
    let preview = DiffPreview {
        file_path: display_path.clone(),
        diff_text: diff.render(),
        diff_colored: diff.render_colored(),
        summary: diff.summary(),
        additions: diff.additions,
        deletions: diff.deletions,
        is_new_file: false,
    };

    fs::write(&absolute_path, &updated)?;

    let output = EditFileOutput {
        file_path: display_path,
        old_string: effective_old,
        new_string: new_string.to_owned(),
        original_file: original_file.clone(),
        structured_patch: make_patch(&original_file, &updated),
        user_modified: false,
        replace_all,
        git_diff: None,
    };
    Ok((output, preview))
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Try to find `old_string` in the file with normalized whitespace.
/// Returns the actual string from the file that matches.
fn fuzzy_find_match(file_content: &str, old_string: &str) -> Option<String> {
    let normalized_old = normalize_whitespace(old_string);
    if normalized_old.is_empty() {
        return None;
    }

    let file_lines: Vec<&str> = file_content.lines().collect();
    let old_lines: Vec<&str> = old_string.lines().collect();

    if old_lines.is_empty() {
        return None;
    }

    let first_normalized = normalize_whitespace(old_lines[0]);
    if first_normalized.is_empty() {
        return None;
    }

    for start in 0..file_lines.len() {
        if normalize_whitespace(file_lines[start]) != first_normalized {
            continue;
        }

        let end = start + old_lines.len();
        if end > file_lines.len() {
            continue;
        }

        let all_match = old_lines.iter().enumerate().all(|(i, old_line)| {
            normalize_whitespace(file_lines[start + i]) == normalize_whitespace(old_line)
        });

        if all_match {
            let matched: String = file_lines[start..end].join("\n");
            if old_string.ends_with('\n') && !matched.ends_with('\n') {
                return Some(format!("{matched}\n"));
            }
            return Some(matched);
        }
    }

    None
}

/// Normalize whitespace for fuzzy comparison: trim trailing, normalize tabs to spaces.
fn normalize_whitespace(s: &str) -> String {
    s.replace('\t', "    ").trim_end().to_string()
}

/// Find the closest matching block in the file for a helpful error message.
fn find_closest_match(file_content: &str, old_string: &str) -> Option<String> {
    let first_line = old_string.lines().next()?;
    let trimmed = first_line.trim();
    if trimmed.is_empty() {
        return None;
    }

    for line in file_content.lines() {
        if line.trim().contains(trimmed) {
            return Some(line.to_string());
        }
    }

    None
}

fn make_patch(original: &str, updated: &str) -> Vec<StructuredPatchHunk> {
    let mut lines = Vec::new();
    for line in original.lines() {
        lines.push(format!("-{line}"));
    }
    for line in updated.lines() {
        lines.push(format!("+{line}"));
    }

    vec![StructuredPatchHunk {
        old_start: 1,
        old_lines: original.lines().count(),
        new_start: 1,
        new_lines: updated.lines().count(),
        lines,
    }]
}
