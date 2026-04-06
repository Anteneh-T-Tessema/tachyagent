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
    /// Configured via .tachy/config.json — not used by default.
    OpenAiCompat,
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
    /// Strong general-purpose model (Gemma 4 26B MoE, Qwen3:8b, Mistral:7b).
    #[default]
    Standard,
    /// Fast/small edge model (Gemma 4 E4B, Llama3.2:3b).
    Fast,
}

#[derive(Clone)]
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
                let backend =
                    OllamaBackend::new(model_name.to_string(), base_url, effective_tools)
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
        }
    }

    /// Build a default registry — all models run locally via Ollama.
    /// No cloud, no Gemini API. Families: Gemma 4, Qwen3, Llama 3, Mistral.
    /// Default model: gemma4:26b  (recommended — 256K ctx, MoE fast).
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
        assert!(registry.configs.get("gemini").is_none());
        // Every registered model must be Ollama-backed
        for model in registry.list_models() {
            assert_eq!(model.backend, BackendKind::Ollama,
                "cloud model {} must not be registered", model.name);
        }
    }

    #[test]
    fn unknown_model_returns_error() {
        let registry = BackendRegistry::with_defaults();
        assert!(registry.create_client("nonexistent", false).is_err());
    }
}
