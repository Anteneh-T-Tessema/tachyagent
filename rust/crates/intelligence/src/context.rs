use std::path::Path;

use serde::{Deserialize, Serialize};

use backend::EmbeddingClient;
use crate::indexer::{CodebaseIndex, FileEntry, semantic_score};

/// Configuration for context selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    pub max_context_percentage: f32,
    pub max_full_files: usize,
    pub max_summaries: usize,
    pub min_relevance: f32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_percentage: 0.40,
            max_full_files: 5,
            max_summaries: 20,
            min_relevance: 0.1,
        }
    }
}

/// The context injection prepended to the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextInjection {
    pub summaries: Vec<FileSummary>,
    pub file_contents: Vec<FileContent>,
    pub estimated_tokens: usize,
    pub token_budget: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSummary {
    pub path: String,
    pub language: String,
    pub exports: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub estimated_tokens: usize,
}

#[derive(Debug)]
pub enum ContextError {
    NoIndex,
    Io(std::io::Error),
}

/// Smart context selector.
pub struct ContextSelector;

impl ContextSelector {
    /// Select relevant context for a user prompt.
    ///
    /// Uses semantic embedding (cosine similarity) when `nomic-embed-text` is
    /// available in Ollama. Falls back to keyword scoring otherwise.
    pub fn select_context(
        prompt: &str,
        index: &CodebaseIndex,
        workspace_root: &Path,
        model_context_window: usize,
        config: &ContextConfig,
    ) -> Result<ContextInjection, ContextError> {
        let keywords = Self::extract_keywords(prompt);
        let budget = (model_context_window as f32 * config.max_context_percentage) as usize;

        // Attempt to embed the prompt for semantic retrieval.
        // If Ollama is not running, query_embedding is None and we fall back to keywords.
        let query_embedding: Option<Vec<f32>> = EmbeddingClient::try_new()
            .and_then(|client| client.embed(prompt).ok());

        // Score and rank files using semantic + structural signals
        let mut scored: Vec<(&FileEntry, f32)> = index
            .files
            .values()
            .map(|entry| {
                let score = semantic_score(entry, query_embedding.as_deref(), &keywords, prompt);
                (entry, score)
            })
            .filter(|(_, score)| *score >= config.min_relevance)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Build summaries (cheap, always include)
        let summaries: Vec<FileSummary> = scored
            .iter()
            .take(config.max_summaries)
            .map(|(entry, _)| FileSummary {
                path: entry.path.clone(),
                language: entry.language.clone(),
                exports: entry.exports.clone(),
                summary: entry.summary.clone(),
            })
            .collect();

        // Estimate tokens for summaries
        let summary_text = summaries
            .iter()
            .map(|s| format!("- {} [{}] — {} exports: {}", s.path, s.language, s.summary, s.exports.join(", ")))
            .collect::<Vec<_>>()
            .join("\n");
        let summary_tokens = Self::estimate_tokens(&summary_text);

        // Read full file contents for top files within budget
        let mut file_contents = Vec::new();
        let mut used_tokens = summary_tokens;

        for (entry, _) in scored.iter().take(config.max_full_files) {
            let remaining = budget.saturating_sub(used_tokens);
            if remaining < 100 {
                break;
            }

            let file_path = workspace_root.join(&entry.path);
            let content = match std::fs::read_to_string(&file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let content_tokens = Self::estimate_tokens(&content);
            let (final_content, truncated) = if content_tokens > remaining {
                let max_chars = remaining * 4;
                (content[..max_chars.min(content.len())].to_string(), true)
            } else {
                (content, false)
            };

            let tokens = Self::estimate_tokens(&final_content);
            used_tokens += tokens;

            file_contents.push(FileContent {
                path: entry.path.clone(),
                content: final_content,
                truncated,
                estimated_tokens: tokens,
            });
        }

        Ok(ContextInjection {
            summaries,
            file_contents,
            estimated_tokens: used_tokens,
            token_budget: budget,
        })
    }

    /// Render the injection as a system prompt section.
    pub fn render_injection(injection: &ContextInjection, index: &CodebaseIndex) -> String {
        let mut sections = vec!["# Codebase Context\n".to_string()];

        // Project info
        if let Some(lang) = &index.project.primary_language {
            sections.push(format!(
                "Project: {} files, primary language: {lang}",
                index.project.total_files
            ));
        }
        if let Some(cmd) = &index.project.test_command {
            sections.push(format!("Test command: {cmd}"));
        }

        // Summaries
        if !injection.summaries.is_empty() {
            sections.push("\n## Relevant Files".to_string());
            for s in &injection.summaries {
                let exports = if s.exports.is_empty() {
                    String::new()
                } else {
                    format!(" exports: {}", s.exports.join(", "))
                };
                sections.push(format!("- {} [{}] — {}{}", s.path, s.language, s.summary, exports));
            }
        }

        // File contents
        for fc in &injection.file_contents {
            let truncated = if fc.truncated { " (truncated)" } else { "" };
            sections.push(format!("\n### {}{}\n```\n{}\n```", fc.path, truncated, fc.content));
        }

        sections.join("\n")
    }

    fn extract_keywords(prompt: &str) -> Vec<String> {
        let mut keywords = Vec::new();

        for word in prompt.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/') {
            let w = word.trim();
            if w.len() >= 3 && !is_stop_word(w) {
                keywords.push(w.to_lowercase());
            }
        }

        keywords.dedup();
        keywords
    }


    fn estimate_tokens(text: &str) -> usize {
        text.len() / 4
    }
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the" | "and" | "for" | "are" | "but" | "not" | "you" | "all"
            | "can" | "had" | "her" | "was" | "one" | "our" | "out"
            | "has" | "have" | "from" | "this" | "that" | "with" | "what"
            | "how" | "use" | "file" | "code" | "please" | "help"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use crate::indexer::ProjectMeta;

    fn test_index() -> CodebaseIndex {
        let mut files = BTreeMap::new();
        files.insert("src/auth.rs".to_string(), FileEntry {
            path: "src/auth.rs".to_string(),
            language: "rust".to_string(),
            size: 2048,
            lines: 85,
            exports: vec!["authenticate".to_string(), "verify_token".to_string()],
            summary: "Authentication module with JWT handling".to_string(),
            content_hash: "abc123".to_string(),
            embedding: None,
        });
        files.insert("src/db.rs".to_string(), FileEntry {
            path: "src/db.rs".to_string(),
            language: "rust".to_string(),
            size: 3000,
            lines: 120,
            exports: vec!["Pool".to_string(), "query".to_string()],
            summary: "Database connection pool".to_string(),
            content_hash: "def456".to_string(),
            embedding: None,
        });
        CodebaseIndex {
            version: 1,
            workspace_root: "/tmp/test".to_string(),
            built_at: 0,
            files,
            project: ProjectMeta {
                primary_language: Some("rust".to_string()),
                test_command: Some("cargo test".to_string()),
                build_system: Some("cargo".to_string()),
                total_files: 2,
                total_lines: 205,
            },
        }
    }

    #[test]
    fn context_respects_count_limits() {
        let config = ContextConfig {
            max_full_files: 1,
            max_summaries: 1,
            ..ContextConfig::default()
        };
        let index = test_index();
        let injection = ContextSelector::select_context(
            "auth",
            &index,
            Path::new("/tmp/test"),
            8192,
            &config,
        ).unwrap();
        assert!(injection.summaries.len() <= 1);
        assert!(injection.file_contents.len() <= 1);
    }

    #[test]
    fn context_respects_token_budget() {
        let config = ContextConfig::default();
        let index = test_index();
        let injection = ContextSelector::select_context(
            "auth",
            &index,
            Path::new("/tmp/test"),
            8192,
            &config,
        ).unwrap();
        let budget = (8192.0 * config.max_context_percentage) as usize;
        assert!(injection.estimated_tokens <= budget);
    }

    #[test]
    fn extract_keywords_filters_stop_words() {
        let keywords = ContextSelector::extract_keywords("how to fix the authentication bug");
        assert!(keywords.contains(&"fix".to_string()));
        assert!(keywords.contains(&"authentication".to_string()));
        assert!(keywords.contains(&"bug".to_string()));
        assert!(!keywords.contains(&"the".to_string()));
        assert!(!keywords.contains(&"how".to_string()));
    }
}
