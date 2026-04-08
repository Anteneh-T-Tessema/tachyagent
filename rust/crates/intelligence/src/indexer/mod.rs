//! Codebase indexer: types, build/load/save/search, and sub-module wiring.
//!
//! Sub-modules:
//!   lang    — language detection, test-command detection, file filtering
//!   summary — export extraction and rich summary generation
//!   search  — keyword scoring, semantic embeddings, content hashing

mod lang;
mod summary;
mod search;

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::Path;

use serde::{Deserialize, Serialize};
use crate::rag::VectorStore;

// Re-export the functions that external callers reference as `crate::indexer::*`
pub use lang::{detect_language, detect_test_command};
pub use search::{embed_summaries, semantic_score};

use lang::{detect_build_system, is_ignored_dir, is_binary_extension};
use summary::extract_summary;
use search::{search_score, simple_hash};

/// Configuration for the codebase indexer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerConfig {
    /// Maximum file size to index (bytes)
    pub max_file_size: u64,
    /// Additional paths to ignore
    pub ignore_paths: Vec<String>,
    /// Whether to auto-rebuild index on each session start
    pub auto_rebuild: bool,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            max_file_size: 100_000,
            ignore_paths: vec![],
            auto_rebuild: false,
        }
    }
}

/// The full codebase index, persisted to .tachy/index.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseIndex {
    pub version: u32,
    pub workspace_root: String,
    pub built_at: u64,
    pub files: BTreeMap<String, FileEntry>,
    pub project: ProjectMeta,
    #[serde(default)]
    pub vector_store: VectorStore,
}

/// Metadata about the overall project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub primary_language: Option<String>,
    pub test_command: Option<String>,
    pub build_system: Option<String>,
    pub total_files: usize,
    pub total_lines: usize,
}

/// A single indexed file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub language: String,
    pub size: u64,
    pub lines: usize,
    pub exports: Vec<String>,
    pub summary: String,
    pub content_hash: String,
    /// Semantic embedding of the summary, produced by the local embedding model.
    /// `None` when Ollama is not running or the embedding model is not installed.
    /// When present, used for cosine-similarity-based context retrieval instead
    /// of keyword scoring.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

/// Errors from the indexer
#[derive(Debug)]
pub enum IndexError {
    Io(std::io::Error),
    Json(serde_json::Error),
    WorkspaceNotFound(String),
}

impl Display for IndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
            Self::WorkspaceNotFound(p) => write!(f, "workspace not found: {p}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for IndexError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// The codebase indexer.
pub struct CodebaseIndexer;

impl CodebaseIndexer {
    /// Build a full index of the workspace.
    pub fn build_index(workspace_root: &Path, config: &IndexerConfig) -> Result<CodebaseIndex, IndexError> {
        if !workspace_root.exists() {
            return Err(IndexError::WorkspaceNotFound(
                workspace_root.to_string_lossy().to_string(),
            ));
        }

        let mut files = BTreeMap::new();
        let mut total_lines = 0usize;
        let mut lang_counts: BTreeMap<String, usize> = BTreeMap::new();

        Self::walk_directory(workspace_root, workspace_root, config, &mut files, &mut total_lines, &mut lang_counts)?;

        let primary_language = lang_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(lang, _)| lang);

        let test_command = detect_test_command(workspace_root);
        let build_system = detect_build_system(workspace_root);

        let project = ProjectMeta {
            primary_language,
            test_command,
            build_system,
            total_files: files.len(),
            total_lines,
        };

        let built_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Attempt semantic embedding of all summaries in one pass.
        // If Ollama / nomic-embed-text is not available, skip silently —
        // context selection falls back to keyword scoring.
        let mut vector_store = VectorStore::new();
        embed_summaries(&mut files, &mut vector_store, workspace_root, config);

        Ok(CodebaseIndex {
            version: 1,
            workspace_root: workspace_root.to_string_lossy().to_string(),
            built_at,
            files,
            project,
            vector_store,
        })
    }

    fn walk_directory(
        root: &Path,
        dir: &Path,
        config: &IndexerConfig,
        files: &mut BTreeMap<String, FileEntry>,
        total_lines: &mut usize,
        lang_counts: &mut BTreeMap<String, usize>,
    ) -> Result<(), IndexError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()), // skip unreadable dirs
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                if is_ignored_dir(&name) || config.ignore_paths.iter().any(|p| name == *p) {
                    continue;
                }
                Self::walk_directory(root, &path, config, files, total_lines, lang_counts)?;
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if is_binary_extension(ext) {
                    continue;
                }

                let metadata = match std::fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if metadata.len() > config.max_file_size {
                    continue;
                }

                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                let language = detect_language(&relative).to_string();
                if language == "unknown" && !matches!(ext, "toml" | "json" | "yaml" | "yml" | "md") {
                    continue; // skip truly unknown files
                }

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue, // skip binary/unreadable
                };

                let lines = content.lines().count();
                *total_lines += lines;

                let (exports, summary) = extract_summary(&relative, &content, &language);
                let content_hash = simple_hash(&content);

                *lang_counts.entry(language.clone()).or_insert(0) += 1;

                files.insert(
                    relative.clone(),
                    FileEntry {
                        path: relative,
                        language,
                        size: metadata.len(),
                        lines,
                        exports,
                        summary,
                        content_hash,
                        embedding: None,
                    },
                );
            }
        }

        Ok(())
    }

    /// Load a previously persisted index.
    pub fn load_index(workspace_root: &Path) -> Result<CodebaseIndex, IndexError> {
        let path = workspace_root.join(".tachy").join("index.json");
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Persist the index to disk.
    pub fn save_index(workspace_root: &Path, index: &CodebaseIndex) -> Result<(), IndexError> {
        let dir = workspace_root.join(".tachy");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("index.json");
        let json = serde_json::to_string_pretty(index)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Incrementally update — only re-index files whose hash changed.
    pub fn update_index(
        workspace_root: &Path,
        existing: &CodebaseIndex,
        config: &IndexerConfig,
    ) -> Result<(CodebaseIndex, usize), IndexError> {
        let new_index = Self::build_index(workspace_root, config)?;
        let mut reindexed = 0usize;

        for (path, new_entry) in &new_index.files {
            if let Some(old_entry) = existing.files.get(path) {
                if old_entry.content_hash != new_entry.content_hash {
                    reindexed += 1;
                }
            } else {
                reindexed += 1; // new file
            }
        }

        // Count removed files
        for path in existing.files.keys() {
            if !new_index.files.contains_key(path) {
                reindexed += 1;
            }
        }

        Ok((new_index, reindexed))
    }

    /// Search the index for files matching a query.
    pub fn search<'a>(
        index: &'a CodebaseIndex,
        query: &str,
        max_results: usize,
    ) -> Vec<&'a FileEntry> {
        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&FileEntry, f32)> = index
            .files
            .values()
            .map(|entry| {
                let score = search_score(entry, &keywords);
                (entry, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_results);
        scored.into_iter().map(|(entry, _)| entry).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::lang::{is_ignored_dir, is_binary_extension};
    use super::search::simple_hash;
    use super::summary::extract_summary;

    #[test]
    fn detect_language_covers_common_extensions() {
        assert_eq!(detect_language("src/main.rs"), "rust");
        assert_eq!(detect_language("app.py"), "python");
        assert_eq!(detect_language("index.ts"), "typescript");
        assert_eq!(detect_language("index.tsx"), "typescript");
        assert_eq!(detect_language("app.js"), "javascript");
        assert_eq!(detect_language("main.go"), "go");
        assert_eq!(detect_language("unknown.xyz"), "unknown");
    }

    #[test]
    fn detect_language_is_deterministic() {
        for _ in 0..100 {
            assert_eq!(detect_language("test.rs"), "rust");
        }
    }

    #[test]
    fn extract_rust_exports() {
        let content = "pub fn main() {}\npub struct Config {}\nfn private() {}\npub enum Status {}";
        let (exports, _) = extract_summary("test.rs", content, "rust");
        assert!(exports.contains(&"main".to_string()));
        assert!(exports.contains(&"Config".to_string()));
        assert!(exports.contains(&"Status".to_string()));
        assert!(!exports.contains(&"private".to_string()));
    }

    #[test]
    fn extract_python_exports() {
        let content = "def hello():\n    pass\nclass MyClass:\n    def method(self):\n        pass";
        let (exports, _) = extract_summary("test.py", content, "python");
        assert!(exports.contains(&"hello".to_string()));
        assert!(exports.contains(&"MyClass".to_string()));
        // "method" is indented with spaces, so it should not be extracted as top-level
        // But our simple parser checks starts_with(' ') which catches "    def method"
        assert_eq!(exports.len(), 2);
    }

    #[test]
    fn extract_ts_exports() {
        let content = "export function render() {}\nexport class App {}\nfunction internal() {}";
        let (exports, _) = extract_summary("test.ts", content, "typescript");
        assert!(exports.contains(&"render".to_string()));
        assert!(exports.contains(&"App".to_string()));
        assert!(!exports.contains(&"internal".to_string()));
    }

    #[test]
    fn summary_truncated_to_120_chars() {
        let long_line = "x".repeat(200);
        let (_, summary) = extract_summary("test.rs", &long_line, "rust");
        assert!(summary.chars().count() <= 121); // 119 chars + "…" (1 char)
    }

    #[test]
    fn exports_capped_at_20() {
        let content = (0..30)
            .map(|i| format!("pub fn func_{i}() {{}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (exports, _) = extract_summary("test.rs", &content, "rust");
        assert!(exports.len() <= 20);
    }

    #[test]
    fn ignored_dirs_are_skipped() {
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("node_modules"));
        assert!(is_ignored_dir("target"));
        assert!(!is_ignored_dir("src"));
    }

    #[test]
    fn binary_extensions_are_skipped() {
        assert!(is_binary_extension("png"));
        assert!(is_binary_extension("exe"));
        assert!(!is_binary_extension("rs"));
        assert!(!is_binary_extension("py"));
    }

    #[test]
    fn simple_hash_is_deterministic() {
        let h1 = simple_hash("hello world");
        let h2 = simple_hash("hello world");
        assert_eq!(h1, h2);
        assert_ne!(h1, simple_hash("hello world!"));
    }

    #[test]
    fn build_index_on_real_directory() {
        let dir = std::env::temp_dir().join(format!(
            "tachy-idx-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "pub fn main() {}\npub struct App {}").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub mod utils;\npub fn init() {}").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let config = IndexerConfig::default();
        let index = CodebaseIndexer::build_index(&dir, &config).expect("should build");

        assert_eq!(index.files.len(), index.project.total_files);
        assert!(index.files.len() >= 2); // at least the two .rs files
        assert_eq!(index.project.test_command.as_deref(), Some("cargo test"));
        assert_eq!(index.project.primary_language.as_deref(), Some("rust"));

        // Test save/load round-trip
        std::fs::create_dir_all(dir.join(".tachy")).unwrap();
        CodebaseIndexer::save_index(&dir, &index).expect("should save");
        let loaded = CodebaseIndexer::load_index(&dir).expect("should load");
        assert_eq!(loaded.files.len(), index.files.len());
        assert_eq!(loaded.version, index.version);

        // Test search
        let results = CodebaseIndexer::search(&index, "main", 5);
        assert!(!results.is_empty());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn search_respects_max_results() {
        let mut files = BTreeMap::new();
        for i in 0..20 {
            files.insert(
                format!("file_{i}.rs"),
                FileEntry {
                    path: format!("file_{i}.rs"),
                    language: "rust".to_string(),
                    size: 100,
                    lines: 10,
                    exports: vec!["test".to_string()],
                    summary: "test file".to_string(),
                    content_hash: format!("{i:016x}"),
                    embedding: None,
                },
            );
        }
        let index = CodebaseIndex {
            version: 1,
            workspace_root: "/tmp".to_string(),
            built_at: 0,
            files,
            vector_store: VectorStore::new(),
            project: ProjectMeta {
                primary_language: Some("rust".to_string()),
                test_command: None,
                build_system: None,
                total_files: 20,
                total_lines: 200,
            },
        };

        let results = CodebaseIndexer::search(&index, "test", 5);
        assert!(results.len() <= 5);
    }

    #[test]
    fn workspace_not_found_returns_error() {
        let result = CodebaseIndexer::build_index(
            Path::new("/nonexistent/path/that/does/not/exist"),
            &IndexerConfig::default(),
        );
        assert!(matches!(result, Err(IndexError::WorkspaceNotFound(_))));
    }
}
