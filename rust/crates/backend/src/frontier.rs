//! Frontier model client for coordinator-level planning.
//!
//! Used exclusively by the swarm coordinator (DAG planning) — workers always
//! run local Ollama. This split gives maximum reasoning quality for the hard
//! decomposition step while keeping all codebase data on-premise.

use serde::{Deserialize, Serialize};

/// Which frontier provider to use for coordinator planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinatorProvider {
    /// Call Anthropic Claude API (claude-sonnet-4-6 by default).
    Anthropic,
    /// Call any OpenAI-compatible API (`OpenAI`, Together, Fireworks, etc.).
    OpenAi,
    /// Use local Ollama (default, always available, air-gap safe).
    #[default]
    Local,
}

/// Configuration for the swarm coordinator backend.
///
/// Set via environment variables or `coordinator` block in `.tachy/config.json`:
/// - `TACHY_COORDINATOR_PROVIDER` = `anthropic|openai|local`
/// - `TACHY_COORDINATOR_MODEL`    = e.g. `claude-sonnet-4-6`, `gpt-4o`
/// - `TACHY_COORDINATOR_API_KEY`  = API key for the chosen provider
/// - `TACHY_COORDINATOR_BASE_URL` = override base URL (for OpenAI-compat)
/// - `TACHY_AIR_GAP=1`            = force local regardless of config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoordinatorConfig {
    pub provider: CoordinatorProvider,
    pub model: Option<String>,
    pub api_key: Option<String>,
    /// Base URL override for OpenAI-compatible providers.
    pub base_url: Option<String>,
    /// When true, ignore `provider` and always use local Ollama.
    #[serde(default)]
    pub air_gap: bool,
}

impl CoordinatorConfig {
    /// Load from environment variables. Falls back to local if unset.
    #[must_use] pub fn from_env() -> Self {
        let air_gap = std::env::var("TACHY_AIR_GAP")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        if air_gap {
            return Self { air_gap: true, ..Default::default() };
        }

        let provider = match std::env::var("TACHY_COORDINATOR_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "anthropic" => CoordinatorProvider::Anthropic,
            "openai"    => CoordinatorProvider::OpenAi,
            _           => CoordinatorProvider::Local,
        };

        let model     = std::env::var("TACHY_COORDINATOR_MODEL").ok();
        let api_key   = std::env::var("TACHY_COORDINATOR_API_KEY").ok();
        let base_url  = std::env::var("TACHY_COORDINATOR_BASE_URL").ok();

        Self { provider, model, api_key, base_url, air_gap }
    }

    /// Effective provider after air-gap enforcement.
    #[must_use] pub fn effective_provider(&self) -> &CoordinatorProvider {
        if self.air_gap { &CoordinatorProvider::Local } else { &self.provider }
    }

    /// Effective model name for this configuration.
    #[must_use] pub fn effective_model(&self) -> String {
        if self.air_gap {
            return self.model.clone().unwrap_or_else(|| "gemma4:26b".to_string());
        }
        self.model.clone().unwrap_or_else(|| match &self.provider {
            CoordinatorProvider::Anthropic => "claude-sonnet-4-6".to_string(),
            CoordinatorProvider::OpenAi    => "gpt-4o".to_string(),
            CoordinatorProvider::Local     => "gemma4:26b".to_string(),
        })
    }
}

/// A simple blocking HTTP client for frontier planning calls.
/// Only used for DAG decomposition — not for code execution.
pub struct FrontierPlanner {
    config: CoordinatorConfig,
    client: reqwest::blocking::Client,
}

impl FrontierPlanner {
    #[must_use] pub fn new(config: CoordinatorConfig) -> Self {
        Self {
            config,
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Send a planning prompt and return the raw text response.
    /// Returns `None` if the provider is local (caller falls back to Ollama).
    #[must_use] pub fn plan(&self, prompt: &str) -> Option<String> {
        match self.config.effective_provider() {
            CoordinatorProvider::Local => None,
            CoordinatorProvider::Anthropic => self.call_anthropic(prompt),
            CoordinatorProvider::OpenAi    => self.call_openai(prompt),
        }
    }

    fn call_anthropic(&self, prompt: &str) -> Option<String> {
        let api_key = self.config.api_key.as_deref()
            .or({
                // borrow checker: can't use .ok() inside closure that borrows self
                None
            })
            .map(str::to_string)
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())?;

        let model = self.config.effective_model();

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "messages": [{
                "role": "user",
                "content": prompt,
            }],
        });

        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .ok()?;

        if !resp.status().is_success() {
            eprintln!("[coordinator] Anthropic API error: {}", resp.status());
            return None;
        }

        let json: serde_json::Value = resp.json().ok()?;
        json["content"]
            .as_array()?
            .iter()
            .find(|b| b["type"] == "text")?
            ["text"]
            .as_str()
            .map(str::to_string)
    }

    fn call_openai(&self, prompt: &str) -> Option<String> {
        let api_key = self.config.api_key.clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())?;

        let base = self.config.base_url.as_deref()
            .unwrap_or("https://api.openai.com");
        let url = format!("{base}/v1/chat/completions");
        let model = self.config.effective_model();

        let body = serde_json::json!({
            "model": model,
            "messages": [{
                "role": "user",
                "content": prompt,
            }],
            "max_tokens": 4096,
            "temperature": 0.2,
        });

        let resp = self.client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .ok()?;

        if !resp.status().is_success() {
            eprintln!("[coordinator] OpenAI API error: {}", resp.status());
            return None;
        }

        let json: serde_json::Value = resp.json().ok()?;
        json["choices"]
            .as_array()?
            .first()?
            ["message"]["content"]
            .as_str()
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_gap_forces_local() {
        let cfg = CoordinatorConfig {
            provider: CoordinatorProvider::Anthropic,
            air_gap: true,
            ..Default::default()
        };
        assert_eq!(cfg.effective_provider(), &CoordinatorProvider::Local);
    }

    #[test]
    fn default_models_per_provider() {
        let anthropic = CoordinatorConfig {
            provider: CoordinatorProvider::Anthropic,
            ..Default::default()
        };
        assert_eq!(anthropic.effective_model(), "claude-sonnet-4-6");

        let openai = CoordinatorConfig {
            provider: CoordinatorProvider::OpenAi,
            ..Default::default()
        };
        assert_eq!(openai.effective_model(), "gpt-4o");

        let local = CoordinatorConfig::default();
        assert_eq!(local.effective_model(), "gemma4:26b");
    }

    #[test]
    fn custom_model_overrides_default() {
        let cfg = CoordinatorConfig {
            provider: CoordinatorProvider::Anthropic,
            model: Some("claude-opus-4-6".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.effective_model(), "claude-opus-4-6");
    }

    #[test]
    fn plan_returns_none_for_local_provider() {
        let cfg = CoordinatorConfig::default(); // Local
        let planner = FrontierPlanner::new(cfg);
        assert!(planner.plan("test").is_none());
    }
}
