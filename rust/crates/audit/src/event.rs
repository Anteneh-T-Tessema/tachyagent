use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventKind {
    SessionStart,
    SessionEnd,
    UserMessage,
    AssistantMessage,
    ToolInvocation,
    ToolResult,
    PermissionGranted,
    PermissionDenied,
    GovernanceViolation,
    SessionCompacted,
    ConfigChange,
    ModelSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: String,
    pub session_id: String,
    pub kind: AuditEventKind,
    pub severity: AuditSeverity,
    pub agent_id: Option<String>,
    pub user_id: Option<String>,
    pub tool_name: Option<String>,
    pub model_name: Option<String>,
    pub detail: String,
    /// Redacted input/output for compliance — never store raw sensitive data.
    pub redacted_payload: Option<String>,
}

impl AuditEvent {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        kind: AuditEventKind,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: now_iso8601(),
            session_id: session_id.into(),
            kind,
            severity: AuditSeverity::Info,
            agent_id: None,
            user_id: None,
            tool_name: None,
            model_name: None,
            detail: detail.into(),
            redacted_payload: None,
        }
    }

    #[must_use]
    pub fn with_severity(mut self, severity: AuditSeverity) -> Self {
        self.severity = severity;
        self
    }

    #[must_use]
    pub fn with_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    #[must_use]
    pub fn with_model(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = Some(model_name.into());
        self
    }

    #[must_use]
    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    #[must_use]
    pub fn with_user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    #[must_use]
    pub fn with_redacted_payload(mut self, payload: impl Into<String>) -> Self {
        self.redacted_payload = Some(payload.into());
        self
    }

    #[must_use]
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{{\"error\":\"serialize_failed\",\"detail\":\"{}\"}}", self.detail))
    }
}

fn now_iso8601() -> String {
    // Simple timestamp without external crate — seconds since epoch
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_audit_event_with_builder() {
        let event = AuditEvent::new("sess-1", AuditEventKind::ToolInvocation, "executed bash")
            .with_severity(AuditSeverity::Warning)
            .with_tool("bash")
            .with_model("qwen2.5-coder:7b")
            .with_agent("code-reviewer")
            .with_user("user-42");

        assert_eq!(event.session_id, "sess-1");
        assert_eq!(event.tool_name.as_deref(), Some("bash"));
        assert_eq!(event.severity, AuditSeverity::Warning);

        let json = event.to_json_line();
        assert!(json.contains("tool_invocation"));
        assert!(json.contains("bash"));
    }
}
