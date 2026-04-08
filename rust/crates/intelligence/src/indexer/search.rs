//! Search scoring, semantic embeddings, and content hashing.

use std::collections::BTreeMap;
use std::path::Path;

use backend::{cosine_similarity, EmbeddingClient};
use crate::rag::{VectorStore, CodeChunk, Chunker};

use super::{FileEntry, IndexerConfig};

pub(crate) fn search_score(entry: &FileEntry, keywords: &[&str]) -> f32 {
    let mut score = 0.0f32;
    let path_lower = entry.path.to_lowercase();
    let summary_lower = entry.summary.to_lowercase();

    for keyword in keywords {
        if path_lower.contains(keyword) {
            score += 1.0;
        }
        for export in &entry.exports {
            if export.to_lowercase().contains(keyword) {
                score += 0.5;
            }
        }
        if summary_lower.contains(keyword) {
            score += 0.2;
        }
    }

    score
}

/// Embed all file summaries and individual code chunks using the local Ollama embedding model.
/// Silently does nothing if Ollama / the embedding model is unavailable.
pub fn embed_summaries(
    files: &mut BTreeMap<String, FileEntry>,
    vector_store: &mut VectorStore,
    root: &Path,
    _config: &IndexerConfig,
) {
    // Only attempt if Ollama is reachable
    let Some(client) = EmbeddingClient::try_new() else {
        return;
    };

    let chunker = Chunker::default();

    for (path, entry) in files.iter_mut() {
        // 1. Embed Summary (if missing)
        if entry.embedding.is_none() {
            if let Ok(emb) = client.embed(&entry.summary) {
                entry.embedding = Some(emb);
            }
        }

        // 2. Embed Chunks — incremental: skip files whose content hash is
        //    already represented in the vector store to avoid redundant work.
        let already_embedded = vector_store.chunks.iter()
            .any(|c| c.path == *path && c.content_hash == entry.content_hash);
        if already_embedded { continue; }

        // Remove stale chunks for this path before adding fresh ones
        vector_store.chunks.retain(|c| c.path != *path);

        let full_path = root.join(path);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            let chunks = chunker.chunk_file(path, &content);
            for (start, end, text) in chunks {
                if let Ok(emb) = client.embed(&text) {
                    vector_store.add_chunk(CodeChunk {
                        id: format!("{path}:{start}-{end}"),
                        path: path.clone(),
                        start_line: start,
                        end_line: end,
                        content: text,
                        embedding: emb,
                        content_hash: entry.content_hash.clone(),
                    });
                }
            }
        }
    }
}

/// Compute a semantic score for a file given a pre-embedded query vector.
/// Falls back to keyword scoring when embeddings are unavailable.
#[must_use] pub fn semantic_score(entry: &FileEntry, query_embedding: Option<&[f32]>, keywords: &[String], prompt: &str) -> f32 {
    let mut score = 0.0f32;

    // Semantic similarity — dominant signal when available
    if let (Some(file_emb), Some(query_emb)) = (&entry.embedding, query_embedding) {
        let sim = cosine_similarity(file_emb, query_emb);
        score += sim * 3.0;
    }

    let prompt_lower = prompt.to_lowercase();

    // Structural signals — always applied as tiebreakers
    if prompt_lower.contains(&entry.path.to_lowercase()) {
        score += 1.0;
    }
    if let Some(filename) = entry.path.rsplit('/').next() {
        if prompt_lower.contains(&filename.to_lowercase()) {
            score += 0.7;
        }
    }
    for export in &entry.exports {
        if keywords.iter().any(|k| k.eq_ignore_ascii_case(export)) {
            score += 0.5;
        }
    }
    // Summary keyword overlap (kept as weak signal even with embeddings)
    let summary_lower = entry.summary.to_lowercase();
    for keyword in keywords {
        if summary_lower.contains(keyword.as_str()) {
            score += 0.1;
        }
    }
    // Penalise very large files slightly
    if entry.lines > 500 { score *= 0.9; }
    if entry.lines > 1000 { score *= 0.8; }

    score
}

#[allow(clippy::unreadable_literal)]
pub(crate) fn simple_hash(content: &str) -> String {
    // Simple FNV-1a-like hash for change detection (not cryptographic)
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in content.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
