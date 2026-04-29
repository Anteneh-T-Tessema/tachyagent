use std::sync::{Arc, Mutex};
use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};

/// Result stored in the semantic cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResult {
    pub output: String,
    pub model: String,
    pub timestamp: u64,
    pub embedding: Option<Vec<f32>>,
    /// Condensed tool sequence and reasoning from a successful execution.
    pub expert_trace: Option<String>,
    /// Quality score (0.0 to 1.0) to prioritize high-quality guidance.
    pub reward_score: f32,
}

/// Trait for semantic embedding providers.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, String>;
}

/// A sovereign semantic cache for zero-latency AI responses.
pub struct SemanticCache {
    /// Exact match cache (PromptHash -> Result)
    exact_cache: Arc<Mutex<std::collections::HashMap<String, CachedResult>>>,
    /// Hit counter for metrics
    hits: Arc<Mutex<u64>>,
}

impl SemanticCache {
    pub fn new() -> Self {
        Self {
            exact_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            hits: Arc::new(Mutex::new(0)),
        }
    }

    pub fn load(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let cache: std::collections::HashMap<String, CachedResult> = serde_json::from_str(&content)?;
        Ok(Self {
            exact_cache: Arc::new(Mutex::new(cache)),
            hits: Arc::new(Mutex::new(0)),
        })
    }

    pub fn hits(&self) -> u64 {
        *self.hits.lock().unwrap()
    }

    fn record_hit(&self) {
        let mut h = self.hits.lock().unwrap();
        *h += 1;
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let cache = self.exact_cache.lock().unwrap();
        let content = serde_json::to_string_pretty(&*cache)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Calculate a hash for the prompt and context.
    pub fn hash_prompt(prompt: &str, system_prompt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(prompt.as_bytes());
        hasher.update(system_prompt.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Lookup a result in the cache using exact hash matching.
    pub fn lookup(&self, prompt: &str, system_prompt: &str) -> Option<CachedResult> {
        let hash = Self::hash_prompt(prompt, system_prompt);
        let cache = self.exact_cache.lock().unwrap();
        let res = cache.get(&hash).cloned();
        if res.is_some() {
            self.record_hit();
        }
        res
    }

    /// Lookup a result in the cache using semantic similarity.
    pub fn lookup_semantic(&self, query_emb: &[f32], threshold: f32) -> Option<CachedResult> {
        let cache = self.exact_cache.lock().unwrap();
        let mut best_match: Option<(&CachedResult, f32)> = None;

        for result in cache.values() {
            if let Some(emb) = &result.embedding {
                let sim = cosine_similarity(emb, query_emb);
                if sim >= threshold {
                    if let Some((_, best_sim)) = best_match {
                        if sim > best_sim {
                            best_match = Some((result, sim));
                        }
                    } else {
                        best_match = Some((result, sim));
                    }
                }
            }
        }

        let res = best_match.map(|(r, _)| r.clone());
        if res.is_some() {
            self.record_hit();
        }
        res
    }

    /// Store a result in the cache with optional expert guidance.
    pub fn store(&self, prompt: &str, system_prompt: &str, output: &str, model: &str, 
                 embedding: Option<Vec<f32>>, expert_trace: Option<String>, reward_score: f32) {
        let hash = Self::hash_prompt(prompt, system_prompt);
        let mut cache = self.exact_cache.lock().unwrap();
        cache.insert(hash, CachedResult {
            output: output.to_string(),
            model: model.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            embedding,
            expert_trace,
            reward_score,
        });
    }

    /// Clear the cache.
    pub fn clear(&self) {
        let mut cache = self.exact_cache.lock().unwrap();
        cache.clear();
    }
}

/// Compute the cosine similarity between two vectors.
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

impl Default for SemanticCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticCache {
    /// Update the reward score of a specific cached result (for manual guidance reweighting).
    pub fn reweight(&self, prompt: &str, system_prompt: &str, new_score: f32) -> Result<(), String> {
        let hash = Self::hash_prompt(prompt, system_prompt);
        let mut cache = self.exact_cache.lock().unwrap();
        if let Some(result) = cache.get_mut(&hash) {
            result.reward_score = new_score.clamp(0.0, 1.0);
            Ok(())
        } else {
            Err(format!("Hash {} not found in cache", hash))
        }
    }

    /// Update the reward score of a specific result by its hash.
    pub fn reweight_by_hash(&self, hash: &str, new_score: f32) -> Result<(), String> {
        let mut cache = self.exact_cache.lock().unwrap();
        if let Some(result) = cache.get_mut(hash) {
            result.reward_score = new_score.clamp(0.0, 1.0);
            Ok(())
        } else {
            Err(format!("Hash {} not found in cache", hash))
        }
    }
}
