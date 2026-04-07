//! Agent tools for Semantic Codebase Search (Direction A).
//!
//! This module provides the high-level "Search Codebase" tool that agents use
//! to navigate large-scale repositories with pinpoint semantic accuracy.

use std::path::Path;
use serde::{Deserialize, Serialize};
use backend::EmbeddingClient;
use crate::rag::VectorStore;
use crate::indexer::CodebaseIndexer;

/// Input for the `search_codebase` tool.
#[derive(Debug, Deserialize, Serialize)]
pub struct SearchCodebaseInput {
    pub query: String,
    pub limit: Option<usize>,
}

/// Output for the `search_codebase` tool.
#[derive(Debug, Serialize)]
pub struct SearchCodebaseResult {
    pub results: Vec<SearchResultEntry>,
}

#[derive(Debug, Serialize)]
pub struct SearchResultEntry {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub score: f32,
}


/// Returns the tool specifications for RAG tools.
pub fn rag_tool_specs() -> Vec<tools::ToolSpec> {
    vec![
        tools::ToolSpec {
            name: "search_codebase",
            description: "Search the entire codebase semantically using vector embeddings. Returns the most relevant code chunks, their paths, line ranges, and similarity scores. Use this to find specific logic blocks or understand how features are implemented across multiple files.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The natural language query describing what code you're looking for." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Maximum number of results to return (default: 5)." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        tools::ToolSpec {
            name: "expand_context",
            description: "Get more code context around a specific file and line range. Returns the surrounding lines to provide better local understanding. Use this after finding a relevant hit via search_codebase.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative path to the file." },
                    "start_line": { "type": "integer", "minimum": 1, "description": "The starting line number." },
                    "end_line": { "type": "integer", "minimum": 1, "description": "The ending line number." },
                    "context_lines": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Number of lines of context to include around the range (default: 200)." }
                },
                "required": ["path", "start_line", "end_line"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Execute a semantic search across the entire codebase.
/// 
/// This tool generates an embedding for the user's query and performs a 
/// top-k similarity search across all indexed code chunks.
pub fn execute_search_codebase(
    input: &SearchCodebaseInput,
    workspace_root: &Path,
) -> Result<String, String> {
    let limit = input.limit.unwrap_or(5);

    // 1. Load the codebase index
    let index = CodebaseIndexer::load_index(workspace_root)
        .map_err(|e| format!("failed to load index: {e}"))?;

    // 2. Generate embedding for query
    let client = EmbeddingClient::try_new()
        .ok_or("embedding model not found — run: ollama pull nomic-embed-text")?;
    
    let query_emb = client.embed(&input.query)
        .map_err(|e| format!("failed to embed query: {e}"))?;

    // 3. Perform semantic search
    let scored_chunks = index.vector_store.search(&query_emb, limit);

    // 4. Format results
    let results = scored_chunks.into_iter()
        .map(|(chunk, score)| SearchResultEntry {
            path: chunk.path.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            content: chunk.content.clone(),
            score,
        })
        .collect();

    serde_json::to_string_pretty(&SearchCodebaseResult { results })
        .map_err(|e| e.to_string())
}

/// Input for the `expand_context` tool.
#[derive(Debug, Deserialize, Serialize)]
pub struct ExpandContextInput {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub context_lines: Option<usize>,
}

/// Execute a context expansion for a specific code location.
pub fn execute_expand_context(
    input: &ExpandContextInput,
    workspace_root: &Path,
) -> Result<String, String> {
    let context = input.context_lines.unwrap_or(200);
    
    // Read the file content
    let full_path = workspace_root.join(&input.path);
    let content = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("failed to read file: {e}"))?;

    let lines: Vec<&str> = content.lines().collect();
    
    // Calculate new range with context
    let start = input.start_line.saturating_sub(context).max(1);
    let end = (input.end_line + context).min(lines.len());

    // Extract the window
    let window = lines[start - 1..end].join("\n");

    Ok(format!(
        "--- File: {} (Lines {} - {}) ---\n\n{}",
        input.path, start, end, window
    ))
}
