//! Multi-file edit transactions — atomic apply/rollback for agent edits.
//!
//! When an agent edits multiple files in one step, all edits should either
//! succeed together or be rolled back together. This prevents partial
//! application that leaves the codebase in a broken state.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A pending file edit within a transaction.
#[derive(Debug, Clone)]
pub struct PendingEdit {
    pub path: PathBuf,
    pub new_content: String,
    pub original_content: Option<String>,
}

/// A multi-file edit transaction.
pub struct EditTransaction {
    edits: Vec<PendingEdit>,
    applied: Vec<PathBuf>,
}

impl EditTransaction {
    #[must_use]
    pub fn new() -> Self {
        Self {
            edits: Vec::new(),
            applied: Vec::new(),
        }
    }

    /// Stage a file write. Does NOT write to disk yet.
    pub fn stage_write(&mut self, path: impl Into<PathBuf>, new_content: String) {
        let path = path.into();
        let original = fs::read_to_string(&path).ok();
        self.edits.push(PendingEdit {
            path,
            new_content,
            original_content: original,
        });
    }

    /// Stage a file edit (replace `old_string` with `new_string`). Does NOT write to disk yet.
    pub fn stage_edit(
        &mut self,
        path: impl Into<PathBuf>,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<(), io::Error> {
        let path = path.into();
        let original = fs::read_to_string(&path)?;

        if !original.contains(old_string) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "old_string not found",
            ));
        }
        let new_content = if replace_all {
            original.replace(old_string, new_string)
        } else {
            original.replacen(old_string, new_string, 1)
        };
        self.edits.push(PendingEdit {
            path,
            new_content,
            original_content: Some(original),
        });
        Ok(())
    }

    /// Number of staged edits.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edits.len()
    }

    /// Whether the transaction is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// List the files that will be modified.
    #[must_use]
    pub fn files(&self) -> Vec<&Path> {
        self.edits.iter().map(|e| e.path.as_path()).collect()
    }

    /// Apply all edits atomically. If any write fails, roll back all changes.
    pub fn commit(&mut self) -> Result<usize, TransactionError> {
        self.applied.clear();

        for i in 0..self.edits.len() {
            let edit = &self.edits[i];
            let path = edit.path.clone();
            let content = edit.new_content.clone();

            // Create parent directories if needed
            if let Some(parent) = path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    let rolled_back = self.applied.len();
                    self.rollback();
                    return Err(TransactionError::WriteFailed {
                        path,
                        error: e.to_string(),
                        rolled_back,
                    });
                }
            }

            if let Err(e) = fs::write(&path, &content) {
                let rolled_back = self.applied.len();
                self.rollback();
                return Err(TransactionError::WriteFailed {
                    path,
                    error: e.to_string(),
                    rolled_back,
                });
            }
            self.applied.push(path);
        }

        Ok(self.applied.len())
    }

    /// Roll back all applied edits to their original content.
    pub fn rollback(&mut self) {
        for path in &self.applied {
            if let Some(edit) = self.edits.iter().find(|e| e.path == *path) {
                match &edit.original_content {
                    Some(original) => {
                        let _ = fs::write(path, original);
                    }
                    None => {
                        let _ = fs::remove_file(path);
                    } // was a new file
                }
            }
        }
        self.applied.clear();
    }
}

impl Default for EditTransaction {
    fn default() -> Self {
        Self::new()
    }
}

/// Error from a transaction commit.
#[derive(Debug)]
pub enum TransactionError {
    WriteFailed {
        path: PathBuf,
        error: String,
        rolled_back: usize,
    },
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriteFailed {
                path,
                error,
                rolled_back,
            } => {
                write!(
                    f,
                    "transaction failed writing {}: {} ({} files rolled back)",
                    path.display(),
                    error,
                    rolled_back
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachy-txn-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn commit_writes_all_files() {
        let dir = temp_dir();
        let f1 = dir.join("a.txt");
        let f2 = dir.join("b.txt");

        let mut txn = EditTransaction::new();
        txn.stage_write(f1.clone(), "content-a".to_string());
        txn.stage_write(f2.clone(), "content-b".to_string());

        let count = txn.commit().unwrap();
        assert_eq!(count, 2);
        assert_eq!(fs::read_to_string(&f1).unwrap(), "content-a");
        assert_eq!(fs::read_to_string(&f2).unwrap(), "content-b");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rollback_restores_originals() {
        let dir = temp_dir();
        let f1 = dir.join("existing.txt");
        fs::write(&f1, "original").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(f1.clone(), "modified".to_string());
        txn.commit().unwrap();
        assert_eq!(fs::read_to_string(&f1).unwrap(), "modified");

        // Manually rollback
        txn.rollback();
        assert_eq!(fs::read_to_string(&f1).unwrap(), "original");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn stage_edit_replaces_text() {
        let dir = temp_dir();
        let f = dir.join("code.rs");
        fs::write(&f, "fn old() {}").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_edit(&f, "old", "new", false).unwrap();
        txn.commit().unwrap();

        assert_eq!(fs::read_to_string(&f).unwrap(), "fn new() {}");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn new_files_deleted_on_rollback() {
        let dir = temp_dir();
        let f = dir.join("new_file.txt");
        assert!(!f.exists());

        let mut txn = EditTransaction::new();
        txn.stage_write(f.clone(), "new content".to_string());
        txn.commit().unwrap();
        assert!(f.exists());

        txn.rollback();
        assert!(!f.exists());

        fs::remove_dir_all(dir).ok();
    }
}
