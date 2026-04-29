use runtime::{ApiClient, ApiRequest, AssistantEvent, RuntimeError};
use serde::{Deserialize, Serialize};

/// A backend that proxies requests to a remote Tachy instance.
pub struct RemoteTachyBackend {
    pub model: String,
    pub remote_url: String,
    pub api_key: Option<String>,
}

impl RemoteTachyBackend {
    pub fn new(model: String, remote_url: String, api_key: Option<String>) -> Self {
        Self {
            model,
            remote_url,
            api_key,
        }
    }
}

impl ApiClient for RemoteTachyBackend {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        // Forward the request to the remote Tachy node's delegation endpoint.
        // For Phase 23, we use a simple POST. In later phases, we'll use streaming.
        let client = reqwest::blocking::Client::new();
        
        let mut req_builder = client.post(format!("{}/api/swarm/execute", self.remote_url))
            .json(&request);
            
        if let Some(ref key) = self.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
        }

        let response = req_builder.send()
            .map_err(|e| RuntimeError::new(format!("Remote Tachy connection failed: {e}")))?;

        if !response.status().is_success() {
            return Err(RuntimeError::new(format!("Remote Tachy returned error: {}", response.status())));
        }

        let events: Vec<AssistantEvent> = response.json()
            .map_err(|e| RuntimeError::new(format!("Failed to parse remote response: {e}")))?;

        Ok(events)
    }
}
