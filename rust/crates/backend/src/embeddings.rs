//! Semantic embedding client for local Ollama embedding models.
//!
//! Used by the intelligence layer for cosine-similarity-based context retrieval.
//! Falls back gracefully when Ollama is not running or the embedding model is not installed.
//!
//! Recommended model: `nomic-embed-text` (pull with `ollama pull nomic-embed-text`)
//! Alternative: `mxbai-embed-large`, `all-minilm`

/// Default Ollama base URL.
const DEFAULT_BASE_URL: &str = "http://localhost:11434";
/// Default embedding model — small, fast, 768-dim, runs on CPU.
const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
/// HTTP timeout for embedding requests (ms). Embedding is fast — 30s is generous.
const EMBED_TIMEOUT_SECS: u64 = 30;

/// Error variants from the embedding client.
#[derive(Debug, Clone)]
pub enum EmbeddingError {
    /// Ollama is not reachable.
    OllamaUnreachable,
    /// The embedding model is not installed in Ollama.
    ModelNotFound(String),
    /// Ollama returned an unexpected response format.
    BadResponse(String),
    /// HTTP or I/O error.
    Http(String),
}

impl std::fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OllamaUnreachable => write!(f, "Ollama not reachable at {DEFAULT_BASE_URL}"),
            Self::ModelNotFound(m) => write!(f, "embedding model '{m}' not found — run: ollama pull {m}"),
            Self::BadResponse(r) => write!(f, "unexpected Ollama response: {r}"),
            Self::Http(e) => write!(f, "HTTP error: {e}"),
        }
    }
}

/// Synchronous embedding client wrapping Ollama's `/api/embeddings` endpoint.
///
/// Intentionally synchronous (blocking) so it integrates cleanly with the
/// synchronous indexer path without requiring an async runtime.
pub struct EmbeddingClient {
    base_url: String,
    model: String,
    http: reqwest::blocking::Client,
}

impl EmbeddingClient {
    /// Create a client pointing at the default local Ollama instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_EMBED_MODEL.to_string(),
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Create a client with a custom base URL and model name.
    #[must_use]
    pub fn with_config(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Try to create a client, checking Ollama reachability first.
    /// Returns `None` if Ollama is not running — caller falls back to keyword scoring.
    #[must_use] pub fn try_new() -> Option<Self> {
        let client = Self::new();
        if client.is_reachable() { Some(client) } else { None }
    }

    /// Probe the Ollama health endpoint.
    #[must_use] pub fn is_reachable(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.http
            .head(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Embed a text string. Returns a 768-dimensional float vector.
    ///
    /// Uses the integrated reqwest client. We keep embedding calls
    /// explicitly synchronous to simplify indexing logic.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        // Sanitize input — strip newlines, limit length to avoid Ollama OOM
        let text = text.replace('\n', " ");
        let text = if text.len() > 2048 { &text[..2048] } else { &text };

        // Build JSON payload
        let payload = serde_json::json!({
            "model": self.model,
            "prompt": text
        });

        // Call Ollama via reqwest (blocking)
        let url = format!("{}/api/embeddings", self.base_url);
        let response = self.http
            .post(&url)
            .json(&payload)
            .send()
            .map_err(|e| EmbeddingError::Http(e.to_string()))?;

        if !response.status().is_success() {
            return Err(EmbeddingError::OllamaUnreachable);
        }

        let parsed: serde_json::Value = response.json()
            .map_err(|e| EmbeddingError::BadResponse(format!("JSON parse error: {e}")))?;

        // Ollama returns {"error": "..."} when the model is missing
        if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
            if err.contains("not found") || err.contains("pull") {
                return Err(EmbeddingError::ModelNotFound(self.model.clone()));
            }
            return Err(EmbeddingError::BadResponse(err.to_string()));
        }

        let embedding = parsed
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| EmbeddingError::BadResponse("missing 'embedding' field".to_string()))?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect::<Vec<f32>>();

        if embedding.is_empty() {
            return Err(EmbeddingError::BadResponse("empty embedding vector".to_string()));
        }

        Ok(embedding)
    }

    /// Embed a batch of texts. Returns only the successful embeddings paired
    /// with their original index. Failures are silently skipped so a single
    /// bad file doesn't abort the whole index build.
    #[must_use] pub fn embed_batch(&self, texts: &[&str]) -> Vec<(usize, Vec<f32>)> {
        texts
            .iter()
            .enumerate()
            .filter_map(|(i, text)| {
                self.embed(text).ok().map(|emb| (i, emb))
            })
            .collect()
    }
}

impl runtime::Embedder for EmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(text).map_err(|e| e.to_string())
    }
}

impl Default for EmbeddingClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the cosine similarity between two vectors.
/// Returns a value in [0.0, 1.0]. Returns 0.0 for zero-length vectors.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical() {
        let v = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let zero = vec![0.0f32, 0.0];
        let v = vec![1.0f32, 0.0];
        assert_eq!(cosine_similarity(&zero, &v), 0.0);
    }

    #[test]
    fn cosine_similarity_mismatched_lengths() {
        let a = vec![1.0f32, 0.0];
        let b = vec![1.0f32];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_partial_overlap() {
        let a = vec![1.0f32, 1.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.5 && sim < 1.0, "expected partial similarity, got {sim}");
    }
}
