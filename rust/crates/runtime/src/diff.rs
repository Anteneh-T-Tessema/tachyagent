//! Diff preview system — shows changes before applying them.
//!
//! This is a trust boundary: users must see what the agent wants to change
//! before it hits disk. Without this, enterprises won't approve writes.
//!
//! Two modes:
//! - Preview mode: generate diff, return it, don't write
//! - Apply mode: write the change (current behavior)
//!
//! The diff format matches unified diff (compatible with `git apply`).

use std::fmt::Write;

/// A unified diff between two strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedDiff {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Context(String),
    Addition(String),
    Deletion(String),
}

impl UnifiedDiff {
    /// Compute a unified diff between two strings.
    pub fn compute(file_path: &str, old: &str, new: &str) -> Self {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let mut hunks = Vec::new();
        let mut additions = 0usize;
        let mut deletions = 0usize;

        // Simple LCS-based diff (Myers algorithm simplified)
        let edits = compute_edits(&old_lines, &new_lines);

        // Group edits into hunks with 3 lines of context
        let context_lines = 3;
        let mut current_hunk: Option<DiffHunk> = None;
        let mut old_idx = 0usize;
        let mut new_idx = 0usize;

        for edit in &edits {
            match edit {
                Edit::Equal => {
                    if let Some(hunk) = current_hunk.as_mut() {
                        hunk.lines.push(DiffLine::Context(old_lines.get(old_idx).unwrap_or(&"").to_string()));
                        hunk.old_count += 1;
                        hunk.new_count += 1;
                    }
                    old_idx += 1;
                    new_idx += 1;
                }
                Edit::Delete => {
                    if current_hunk.is_none() {
                        let start_old = old_idx.saturating_sub(context_lines);
                        let start_new = new_idx.saturating_sub(context_lines);
                        let mut hunk = DiffHunk {
                            old_start: start_old + 1,
                            old_count: 0,
                            new_start: start_new + 1,
                            new_count: 0,
                            lines: Vec::new(),
                        };
                        // Add context before
                        for i in start_old..old_idx {
                            if i < old_lines.len() {
                                hunk.lines.push(DiffLine::Context(old_lines[i].to_string()));
                                hunk.old_count += 1;
                                hunk.new_count += 1;
                            }
                        }
                        current_hunk = Some(hunk);
                    }
                    let hunk = current_hunk.as_mut().unwrap();
                    hunk.lines.push(DiffLine::Deletion(old_lines.get(old_idx).unwrap_or(&"").to_string()));
                    hunk.old_count += 1;
                    deletions += 1;
                    old_idx += 1;
                }
                Edit::Insert => {
                    if current_hunk.is_none() {
                        let start_old = old_idx.saturating_sub(context_lines);
                        let start_new = new_idx.saturating_sub(context_lines);
                        let mut hunk = DiffHunk {
                            old_start: start_old + 1,
                            old_count: 0,
                            new_start: start_new + 1,
                            new_count: 0,
                            lines: Vec::new(),
                        };
                        for i in start_old..old_idx {
                            if i < old_lines.len() {
                                hunk.lines.push(DiffLine::Context(old_lines[i].to_string()));
                                hunk.old_count += 1;
                                hunk.new_count += 1;
                            }
                        }
                        current_hunk = Some(hunk);
                    }
                    let hunk = current_hunk.as_mut().unwrap();
                    hunk.lines.push(DiffLine::Addition(new_lines.get(new_idx).unwrap_or(&"").to_string()));
                    hunk.new_count += 1;
                    additions += 1;
                    new_idx += 1;
                }
            }
        }

        if let Some(hunk) = current_hunk {
            hunks.push(hunk);
        }

        Self { file_path: file_path.to_string(), hunks, additions, deletions }
    }

    /// Render as unified diff text (compatible with `git apply`).
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "--- a/{}", self.file_path);
        let _ = writeln!(out, "+++ b/{}", self.file_path);

        for hunk in &self.hunks {
            let _ = writeln!(out, "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count);
            for line in &hunk.lines {
                match line {
                    DiffLine::Context(s) => { let _ = writeln!(out, " {s}"); }
                    DiffLine::Addition(s) => { let _ = writeln!(out, "+{s}"); }
                    DiffLine::Deletion(s) => { let _ = writeln!(out, "-{s}"); }
                }
            }
        }
        out
    }

    /// Render as colored terminal output.
    pub fn render_colored(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "\x1b[1m--- a/{}\x1b[0m", self.file_path);
        let _ = writeln!(out, "\x1b[1m+++ b/{}\x1b[0m", self.file_path);

        for hunk in &self.hunks {
            let _ = writeln!(out, "\x1b[36m@@ -{},{} +{},{} @@\x1b[0m",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count);
            for line in &hunk.lines {
                match line {
                    DiffLine::Context(s) => { let _ = writeln!(out, " {s}"); }
                    DiffLine::Addition(s) => { let _ = writeln!(out, "\x1b[32m+{s}\x1b[0m"); }
                    DiffLine::Deletion(s) => { let _ = writeln!(out, "\x1b[31m-{s}\x1b[0m"); }
                }
            }
        }

        let _ = writeln!(out, "\x1b[32m+{}\x1b[0m / \x1b[31m-{}\x1b[0m lines",
            self.additions, self.deletions);
        out
    }

    /// Summary string.
    pub fn summary(&self) -> String {
        format!("{}: +{} -{}", self.file_path, self.additions, self.deletions)
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// Simple edit operations for diff computation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Edit {
    Equal,
    Delete,
    Insert,
}

/// Compute edit operations between two line sequences.
/// Uses a simplified O(NM) algorithm (good enough for files < 10K lines).
fn compute_edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let n = old.len();
    let m = new.len();

    // LCS table
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if old[i - 1] == new[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack to get edits
    let mut edits = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            edits.push(Edit::Equal);
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            edits.push(Edit::Insert);
            j -= 1;
        } else {
            edits.push(Edit::Delete);
            i -= 1;
        }
    }
    edits.reverse();
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_simple_diff() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nmodified\nline3\n";
        let diff = UnifiedDiff::compute("test.txt", old, new);
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.deletions, 1);
        assert!(!diff.is_empty());
    }

    #[test]
    fn renders_unified_format() {
        let old = "a\nb\nc\n";
        let new = "a\nx\nc\n";
        let diff = UnifiedDiff::compute("file.rs", old, new);
        let rendered = diff.render();
        assert!(rendered.contains("--- a/file.rs"));
        assert!(rendered.contains("+++ b/file.rs"));
        assert!(rendered.contains("-b"));
        assert!(rendered.contains("+x"));
    }

    #[test]
    fn empty_diff_for_identical_files() {
        let text = "same\ncontent\n";
        let diff = UnifiedDiff::compute("f.txt", text, text);
        assert!(diff.is_empty());
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.deletions, 0);
    }

    #[test]
    fn handles_additions_only() {
        let old = "a\nb\n";
        let new = "a\nb\nc\nd\n";
        let diff = UnifiedDiff::compute("f.txt", old, new);
        assert_eq!(diff.additions, 2);
        assert_eq!(diff.deletions, 0);
    }

    #[test]
    fn handles_deletions_only() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nb\n";
        let diff = UnifiedDiff::compute("f.txt", old, new);
        assert_eq!(diff.additions, 0);
        assert_eq!(diff.deletions, 2);
    }
}
