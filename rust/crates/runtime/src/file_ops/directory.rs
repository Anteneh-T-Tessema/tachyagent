//! Directory listing with metadata.

use std::io;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::read_write::normalize_path;

/// List directory contents with metadata — essential for project structure understanding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListDirectoryOutput {
    pub path: String,
    pub entries: Vec<DirEntry>,
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

pub fn list_directory(
    path: Option<&str>,
    max_depth: Option<usize>,
) -> io::Result<ListDirectoryOutput> {
    let base = match path {
        Some(p) => normalize_path(p)?,
        None => std::env::current_dir()?,
    };

    if !base.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a directory", base.display()),
        ));
    }

    let depth = max_depth.unwrap_or(1);
    let mut entries = Vec::new();
    let max_entries = 200;

    // Skip common noise directories
    let skip_dirs = [
        "node_modules",
        ".git",
        "target",
        "__pycache__",
        ".venv",
        "venv",
        "dist",
        "build",
        ".next",
        ".cache",
    ];

    let walker = WalkDir::new(&base)
        .min_depth(1)
        .max_depth(depth)
        .sort_by_file_name();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip noisy directories
        if entry.file_type().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if skip_dirs.contains(&name) {
                    continue;
                }
            }
        }

        let name = entry
            .path()
            .strip_prefix(&base)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();

        let size = if entry.file_type().is_file() {
            entry.metadata().ok().map(|m| m.len())
        } else {
            None
        };

        entries.push(DirEntry {
            name,
            is_dir: entry.file_type().is_dir(),
            size,
        });

        if entries.len() >= max_entries {
            break;
        }
    }

    let truncated = entries.len() >= max_entries;
    let total = entries.len();

    Ok(ListDirectoryOutput {
        path: base.to_string_lossy().into_owned(),
        entries,
        total,
        truncated,
    })
}
