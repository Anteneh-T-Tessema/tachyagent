use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use runtime::{ApiClient, ApiRequest, AssistantEvent, RuntimeError};

use crate::ollama::OllamaBackend;
use crate::openai_compat::OpenAiCompatBackend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Ollama,
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
}

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

    /// Build a default registry with common model entries pre-registered.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        registry.register_backend(
            "ollama",
            BackendConfig {
                kind: BackendKind::Ollama,
                base_url: Some("http://localhost:11434".to_string()),
                api_key: None,
                default_model: Some("llama3.1:8b".to_string()),
            },
        );

        // OpenAI-compatible backend — works with OpenAI, Azure OpenAI, Gemini, vLLM, LM Studio
        // Users configure via .tachy/config.json
        registry.register_backend(
            "openai",
            BackendConfig {
                kind: BackendKind::OpenAiCompat,
                base_url: Some("https://api.openai.com/v1".to_string()),
                api_key: None, // set via OPENAI_API_KEY env or config
                default_model: Some("gpt-4o".to_string()),
            },
        );

        // Ollama models — popular coding and general models
        for (name, tool_use, ctx) in [
            ("qwen3:8b", true, 32_768),
            ("qwen3-coder:30b", true, 32_768),
            ("qwen2.5-coder:7b", true, 32_768),
            ("qwen2.5-coder:14b", true, 32_768),
            ("qwen2.5-coder:32b", true, 32_768),
            ("deepseek-coder:latest", false, 16_384),
            ("deepseek-coder-v2:16b", false, 16_384),
            ("llama3.1:8b", true, 131_072),
            ("llama3.1:latest", true, 131_072),
            ("llama3.1:70b", true, 131_072),
            ("llama3.2:3b", true, 131_072),
            ("llama3:latest", true, 8_192),
            ("mistral:7b", true, 32_768),
            ("mistral:latest", true, 32_768),
            ("codellama:7b", false, 16_384),
            ("codestral:22b", true, 32_768),
        ] {
            registry.register_model(ModelEntry {
                name: name.to_string(),
                backend: BackendKind::Ollama,
                supports_tool_use: tool_use,
                context_window: ctx,
                notes: None,
            });
        }

        // Cloud models via OpenAI-compatible API
        // Users need to set OPENAI_API_KEY or configure api_key in .tachy/config.json
        for (name, ctx) in [
            ("gpt-4o", 128_000),
            ("gpt-4o-mini", 128_000),
            ("gpt-4.1", 128_000),
            ("o3-mini", 128_000),
            ("gemini-2.5-pro", 1_000_000),
            ("gemini-2.5-flash", 1_000_000),
        ] {
            registry.register_model(ModelEntry {
                name: name.to_string(),
                backend: BackendKind::OpenAiCompat,
                supports_tool_use: true,
                context_window: ctx,
                notes: Some("cloud model — requires API key".to_string()),
            });
        }

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
    use super::BackendRegistry;

    #[test]
    fn default_registry_has_models() {
        let registry = BackendRegistry::with_defaults();
        assert!(!registry.list_models().is_empty());
        assert!(registry.find_model("qwen2.5-coder:7b").is_some());
        assert!(registry.find_model("llama3.1:8b").is_some());
    }

    #[test]
    fn unknown_model_returns_error() {
        let registry = BackendRegistry::with_defaults();
        let result = registry.create_client("nonexistent", false);
        assert!(result.is_err());
    }
}
