use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use runtime::{ApiClient, ApiRequest, AssistantEvent, RuntimeError};

use crate::ollama::OllamaBackend;
use crate::openai_compat::OpenAiCompatBackend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Ollama,
    /// OpenAI-compatible local server (vLLM, LM Studio, llama.cpp).
    OpenAiCompat,
    /// Another Tachy instance in the swarm.
    RemoteTachy,
    /// Anthropic Claude API — cloud fallback when local Ollama is unreachable.
    /// Uses the Anthropic-compatible `OpenAI` endpoint at api.anthropic.com/v1.
    AnthropicCompat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub kind: BackendKind,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub backend: BackendKind,
    pub supports_tool_use: bool,
    pub context_window: usize,
    pub notes: Option<String>,
    /// Model tier for routing decisions.
    #[serde(default)]
    pub tier: ModelTier,
}

/// Model capability tier — used for intelligent routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// Frontier-class local model (Gemma 4 31B, Qwen3-coder:30b).
    Frontier,
    /// Strong general-purpose model (Gemma 4 26B `MoE`, Qwen3:8b, Mistral:7b).
    #[default]
    Standard,
    /// Fast/small edge model (Gemma 4 E4B, Llama3.2:3b).
    Fast,
}

#[derive(Debug, Clone)]
pub struct BackendRegistry {
    configs: BTreeMap<String, BackendConfig>,
    models: Vec<ModelEntry>,
}

impl BackendRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
            models: Vec::new(),
        }
    }

    pub fn register_backend(&mut self, name: impl Into<String>, config: BackendConfig) {
        self.configs.insert(name.into(), config);
    }

    pub fn register_model(&mut self, entry: ModelEntry) {
        self.models.push(entry);
    }

    #[must_use]
    pub fn list_models(&self) -> &[ModelEntry] {
        &self.models
    }

    #[must_use]
    pub fn find_model(&self, name: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.name == name)
    }

    /// Find the best available frontier-tier model.
    #[must_use]
    pub fn best_frontier_model(&self) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.tier == ModelTier::Frontier)
    }

    /// Find the best available fast-tier model (for simple tool calls).
    #[must_use]
    pub fn best_fast_model(&self) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.tier == ModelTier::Fast)
    }

    /// Create a boxed `ApiClient` for the given model name.
    pub fn create_client(
        &self,
        model_name: &str,
        enable_tools: bool,
    ) -> Result<Box<dyn ApiClient>, RuntimeError> {
        let model_entry = self
            .find_model(model_name)
            .ok_or_else(|| RuntimeError::new(format!("unknown model: {model_name}")))?;

        let config = self
            .configs
            .values()
            .find(|c| c.kind == model_entry.backend)
            .ok_or_else(|| {
                RuntimeError::new(format!(
                    "no backend configured for {:?}",
                    model_entry.backend
                ))
            })?;

        let effective_tools = enable_tools && model_entry.supports_tool_use;

        match config.kind {
            BackendKind::Ollama => {
                let base_url = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                let backend = OllamaBackend::new(model_name.to_string(), base_url, effective_tools)
                    .map_err(|e| RuntimeError::new(e.to_string()))?;
                Ok(Box::new(backend))
            }
            BackendKind::OpenAiCompat => {
                let base_url = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080/v1".to_string());
                let api_key = config.api_key.clone();
                let backend = OpenAiCompatBackend::new(
                    model_name.to_string(),
                    base_url,
                    api_key,
                    effective_tools,
                )
                .map_err(|e| RuntimeError::new(e.to_string()))?;
                Ok(Box::new(backend))
            }
            BackendKind::RemoteTachy => {
                let base_url = config.base_url.clone().unwrap_or_default();
                let api_key = config.api_key.clone();
                let backend =
                    crate::RemoteTachyBackend::new(model_name.to_string(), base_url, api_key);
                Ok(Box::new(backend))
            }
            BackendKind::AnthropicCompat => {
                let base_url = config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
                let api_key = config.api_key.clone();
                let backend = OpenAiCompatBackend::new(
                    model_name.to_string(),
                    base_url,
                    api_key,
                    effective_tools,
                )
                .map_err(|e| RuntimeError::new(e.to_string()))?;
                Ok(Box::new(backend))
            }
        }
    }

    /// Build a registry with Ollama as primary and Anthropic as cloud fallback.
    ///
    /// The cloud fallback is only used when `create_client_with_fallback` is
    /// called and the primary Ollama request fails with a connectivity error.
    /// `with_defaults()` is unchanged — this is a separate opt-in constructor.
    #[must_use]
    pub fn with_cloud_fallback(api_key: Option<String>) -> Self {
        let mut registry = Self::with_defaults();
        let key = api_key.or_else(|| std::env::var("ANTHROPIC_API_KEY").ok());
        if let Some(key) = key {
            registry.register_backend(
                "anthropic",
                BackendConfig {
                    kind: BackendKind::AnthropicCompat,
                    base_url: Some("https://api.anthropic.com/v1".to_string()),
                    api_key: Some(key),
                    default_model: Some("claude-haiku-4-5-20251001".to_string()),
                },
            );
            registry.register_model(ModelEntry {
                name: "claude-haiku-4-5-20251001".to_string(),
                backend: BackendKind::AnthropicCompat,
                supports_tool_use: true,
                context_window: 200_000,
                notes: Some(
                    "Anthropic cloud fallback — active when Ollama unreachable".to_string(),
                ),
                tier: ModelTier::Standard,
            });
        }
        registry
    }

    /// Like `create_client`, but if the primary call fails with a connectivity
    /// error and an `AnthropicCompat` backend is registered, transparently
    /// wraps both into a `FallbackApiClient` that retries on the cloud.
    pub fn create_client_with_fallback(
        &self,
        model_name: &str,
        enable_tools: bool,
    ) -> Result<Box<dyn ApiClient>, RuntimeError> {
        let primary = self.create_client(model_name, enable_tools)?;
        let fallback_entry = self
            .models
            .iter()
            .find(|m| m.backend == BackendKind::AnthropicCompat);

        match fallback_entry {
            Some(fb) => {
                let fb_name = fb.name.clone();
                match self.create_client(&fb_name, enable_tools) {
                    Ok(fallback) => Ok(Box::new(FallbackApiClient {
                        primary,
                        fallback,
                        fallback_model: fb_name,
                    })),
                    Err(_) => Ok(primary),
                }
            }
            None => Ok(primary),
        }
    }

    /// Build a default registry — all models run locally via Ollama.
    /// No cloud, no Gemini API. Families: Gemma 4, Qwen3, Llama 3, Mistral.
    /// Default model: gemma4:26b  (recommended — 256K ctx, `MoE` fast).
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        registry.register_backend(
            "ollama",
            BackendConfig {
                kind: BackendKind::Ollama,
                base_url: Some("http://localhost:11434".to_string()),
                api_key: None,
                default_model: Some("gemma4:26b".to_string()),
            },
        );

        // OpenAI-compatible local server — disabled by default, opt-in via config.
        registry.register_backend(
            "openai-compat",
            BackendConfig {
                kind: BackendKind::OpenAiCompat,
                base_url: Some("http://localhost:8080/v1".to_string()),
                api_key: None,
                default_model: None,
            },
        );

        // ── Frontier ───────────────────────────────────────────────────

        registry.register_model(ModelEntry {
            name: "gemma4:31b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 256_000,
            notes: Some("Gemma 4 31B Dense — best quality, 256K ctx".to_string()),
            tier: ModelTier::Frontier,
        });

        registry.register_model(ModelEntry {
            name: "qwen3-coder:30b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: Some("Qwen3 Coder 30B MoE — coding specialist".to_string()),
            tier: ModelTier::Frontier,
        });

        registry.register_model(ModelEntry {
            name: "qwen2.5-coder:32b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: Some("Qwen2.5 Coder 32B — strong coding, needs ~20GB".to_string()),
            tier: ModelTier::Frontier,
        });

        registry.register_model(ModelEntry {
            name: "llama3.1:70b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 131_072,
            notes: Some("Llama 3.1 70B — needs 40GB+ RAM".to_string()),
            tier: ModelTier::Frontier,
        });

        // ── Standard ───────────────────────────────────────────────────

        registry.register_model(ModelEntry {
            name: "gemma4:26b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 256_000,
            notes: Some("Gemma 4 26B MoE — recommended default, 256K ctx".to_string()),
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "qwen3:8b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: Some("Qwen3 8B — solid general purpose".to_string()),
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "qwen2.5-coder:14b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: None,
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "qwen2.5-coder:7b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: None,
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "mistral:7b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: None,
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "codestral:22b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 32_768,
            notes: Some("Mistral code model — strong at fill-in-the-middle".to_string()),
            tier: ModelTier::Standard,
        });

        registry.register_model(ModelEntry {
            name: "llama3.1:8b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 131_072,
            notes: None,
            tier: ModelTier::Standard,
        });

        // ── Fast / Edge ────────────────────────────────────────────────

        registry.register_model(ModelEntry {
            name: "gemma4:e4b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 128_000,
            notes: Some("Gemma 4 E4B — fast edge model, 128K ctx".to_string()),
            tier: ModelTier::Fast,
        });

        registry.register_model(ModelEntry {
            name: "gemma4:e2b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 128_000,
            notes: Some("Gemma 4 E2B — ultra-fast, minimal RAM, 128K ctx".to_string()),
            tier: ModelTier::Fast,
        });

        registry.register_model(ModelEntry {
            name: "llama3.2:3b".to_string(),
            backend: BackendKind::Ollama,
            supports_tool_use: true,
            context_window: 131_072,
            notes: None,
            tier: ModelTier::Fast,
        });

        registry
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Transparent `ApiClient` wrapper that retries on a cloud backend when the
/// primary (Ollama) returns a connectivity error.
///
/// A connectivity error is identified by checking whether the error message
/// contains "connection refused", "connection reset", "timed out", or "os error".
/// All other errors (invalid model, bad request) propagate immediately without
/// hitting the fallback so you still get fast, meaningful errors during dev.
pub struct FallbackApiClient {
    primary: Box<dyn ApiClient>,
    fallback: Box<dyn ApiClient>,
    fallback_model: String,
}

impl FallbackApiClient {
    pub(crate) fn is_connectivity_error(err: &RuntimeError) -> bool {
        let msg = err.to_string().to_lowercase();
        msg.contains("connection refused")
            || msg.contains("connection reset")
            || msg.contains("timed out")
            || msg.contains("os error")
            || msg.contains("network")
            || msg.contains("connect error")
    }
}

impl ApiClient for FallbackApiClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        match self.primary.stream(request.clone()) {
            Ok(events) => Ok(events),
            Err(err) if Self::is_connectivity_error(&err) => {
                eprintln!(
                    "[FallbackApiClient] Ollama unreachable ({err}); retrying with cloud model '{}'",
                    self.fallback_model
                );
                self.fallback.stream(request)
            }
            Err(err) => Err(err),
        }
    }
}

/// Wrapper so a boxed backend can be used where `ApiClient` is expected.
pub struct DynBackend(Box<dyn ApiClient>);

impl DynBackend {
    #[must_use]
    pub fn new(inner: Box<dyn ApiClient>) -> Self {
        Self(inner)
    }
}

impl ApiClient for DynBackend {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        self.0.stream(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_models() {
        let registry = BackendRegistry::with_defaults();
        assert!(!registry.list_models().is_empty());
        assert!(registry.find_model("gemma4:26b").is_some());
        assert!(registry.find_model("gemma4:31b").is_some());
        assert!(registry.find_model("qwen3-coder:30b").is_some());
        assert!(registry.find_model("llama3.1:8b").is_some());
        assert!(registry.find_model("mistral:7b").is_some());
    }

    #[test]
    fn all_models_use_ollama_backend() {
        let registry = BackendRegistry::with_defaults();
        for model in registry.list_models() {
            assert_eq!(
                model.backend,
                BackendKind::Ollama,
                "model {} must use Ollama — no cloud backends allowed",
                model.name
            );
        }
    }

    #[test]
    fn gemma4_26b_is_default() {
        let registry = BackendRegistry::with_defaults();
        let config = registry.configs.get("ollama").unwrap();
        assert_eq!(config.default_model.as_deref(), Some("gemma4:26b"));
    }

    #[test]
    fn frontier_model_is_gemma4_31b() {
        let registry = BackendRegistry::with_defaults();
        let frontier = registry.best_frontier_model().unwrap();
        assert_eq!(frontier.name, "gemma4:31b");
        assert_eq!(frontier.context_window, 256_000);
    }

    #[test]
    fn fast_model_is_gemma4_e4b() {
        let registry = BackendRegistry::with_defaults();
        let fast = registry.best_fast_model().unwrap();
        assert_eq!(fast.name, "gemma4:e4b");
    }

    #[test]
    fn no_cloud_backends_registered() {
        let registry = BackendRegistry::with_defaults();
        // No Gemini or external cloud backend should exist
        assert!(!registry.configs.contains_key("gemini"));
        // Every registered model must be Ollama-backed
        for model in registry.list_models() {
            assert_eq!(
                model.backend,
                BackendKind::Ollama,
                "cloud model {} must not be registered",
                model.name
            );
        }
    }

    #[test]
    fn unknown_model_returns_error() {
        let registry = BackendRegistry::with_defaults();
        assert!(registry.create_client("nonexistent", false).is_err());
    }

    // --- Cloud fallback tests (do NOT use with_defaults — that must stay sovereign) ---

    #[test]
    fn with_cloud_fallback_no_key_does_not_add_cloud_model() {
        // When no API key is available, with_cloud_fallback behaves like with_defaults.
        // We unset the env var to ensure a clean test.
        // Safety: single-threaded test binary section.
        std::env::remove_var("ANTHROPIC_API_KEY");
        let registry = BackendRegistry::with_cloud_fallback(None);
        // All models must still be Ollama-backed (no key → no cloud registered).
        for model in registry.list_models() {
            assert_eq!(
                model.backend,
                BackendKind::Ollama,
                "expected only Ollama models when no cloud key provided, got: {}",
                model.name
            );
        }
    }

    #[test]
    fn with_cloud_fallback_with_key_registers_anthropic_model() {
        let registry = BackendRegistry::with_cloud_fallback(Some("sk-test-key".to_string()));
        let cloud_models: Vec<_> = registry
            .list_models()
            .iter()
            .filter(|m| m.backend == BackendKind::AnthropicCompat)
            .collect();
        assert_eq!(
            cloud_models.len(),
            1,
            "expected exactly one cloud fallback model"
        );
        assert_eq!(cloud_models[0].name, "claude-haiku-4-5-20251001");
        assert_eq!(cloud_models[0].context_window, 200_000);
        // The default Ollama models must still be present.
        assert!(registry.find_model("gemma4:26b").is_some());
    }

    #[test]
    fn fallback_connectivity_error_detection() {
        use runtime::RuntimeError;
        let transient = RuntimeError::new("connection refused (os error 111)");
        assert!(FallbackApiClient::is_connectivity_error(&transient));
        let hard = RuntimeError::new("invalid model name: foo");
        assert!(!FallbackApiClient::is_connectivity_error(&hard));
    }
}
