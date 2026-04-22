//! Shared response type and lightweight response-payload structs used across
//! all HTTP handler sub-modules.

use serde::Serialize;

// ── Core response type ────────────────────────────────────────────────────────

pub enum Response {
    Full {
        status: u16,
        content_type: String,
        body: String,
    },
    Stream {
        status: u16,
        content_type: String,
        rx: tokio::sync::mpsc::Receiver<String>,
    },
}

impl Response {
    pub fn json(status: u16, body: impl Serialize) -> Self {
        Self::Full {
            status,
            content_type: "application/json".to_string(),
            body: serde_json::to_string(&body).unwrap_or_default(),
        }
    }

    pub fn html(status: u16, body: &str) -> Self {
        Self::Full {
            status,
            content_type: "text/html".to_string(),
            body: body.to_string(),
        }
    }

    pub fn sse() -> (Self, tokio::sync::mpsc::Sender<String>) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        (
            Self::Stream {
                status: 200,
                content_type: "text/event-stream".to_string(),
                rx,
            },
            tx,
        )
    }

    #[cfg(test)]
    pub fn contains(&self, s: &str) -> bool {
        match self {
            Self::Full { body, .. } => body.contains(s),
            Self::Stream { .. } => false,
        }
    }
}

// ── Response payload types ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub models: usize,
    pub agents: usize,
    pub tasks: usize,
    pub workspace: String,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub backend: String,
    pub supports_tool_use: bool,
    pub context_window: usize,
}

#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub template: String,
    pub status: String,
    pub iterations: usize,
    pub tool_invocations: u32,
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub status: String,
    pub run_count: u32,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct TemplateInfo {
    pub name: String,
    pub description: String,
    pub model: String,
    pub tools: Vec<String>,
    pub max_iterations: usize,
    pub requires_approval: bool,
}
