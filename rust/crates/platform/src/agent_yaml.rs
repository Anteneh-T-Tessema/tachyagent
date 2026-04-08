//! User-defined agents loaded from `.tachy/agents/*.yaml`.
//!
//! Example `.tachy/agents/customer-support.yaml`:
//! ```yaml
//! name: customer-support
//! description: Handles customer support tickets
//! model: gemma4:26b
//! system_prompt: |
//!   You are a customer support agent. Be helpful, empathetic, and concise.
//!   Use the query_db tool to look up order information.
//!   Use the send_email tool to respond to customers.
//! tools:
//!   - query_db
//!   - send_email
//!   - read_file
//! max_iterations: 10
//! approval_required: false
//! use_planning: false
//! triggers:
//!   - type: webhook
//!     path: /support
//!   - type: schedule
//!     interval_seconds: 300
//! ```

use std::path::Path;
use serde::{Deserialize, Serialize};

use crate::agent::AgentTemplate;

/// A user-defined agent loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default = "default_max_iter")]
    pub max_iterations: usize,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default = "default_true")]
    pub use_planning: bool,
    #[serde(default)]
    pub triggers: Vec<TriggerDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDef {
    pub r#type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default)]
    pub channel: Option<String>,
}

fn default_model() -> String { "gemma4:26b".to_string() }
fn default_max_iter() -> usize { 16 }
fn default_true() -> bool { true }

impl AgentDefinition {
    /// Convert to an `AgentTemplate` for execution.
    #[must_use] pub fn to_template(&self) -> AgentTemplate {
        AgentTemplate {
            name: self.name.clone(),
            description: if self.description.is_empty() {
                format!("User-defined agent: {}", self.name)
            } else {
                self.description.clone()
            },
            system_prompt: self.system_prompt.clone(),
            allowed_tools: self.tools.clone(),
            model: self.model.clone(),
            max_iterations: self.max_iterations,
            requires_approval: self.approval_required,
            use_planning: self.use_planning,
        }
    }
}

/// Load all agent definitions from `.tachy/agents/`.
#[must_use] pub fn load_agent_definitions(tachy_dir: &Path) -> Vec<AgentDefinition> {
    let agents_dir = tachy_dir.join("agents");
    if !agents_dir.exists() {
        return Vec::new();
    }

    let mut agents = Vec::new();
    let entries = match std::fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            match parse_agent_yaml(&content) {
                Ok(agent) => agents.push(agent),
                Err(e) => {
                    eprintln!("warning: failed to parse {}: {e}", path.display());
                }
            }
        }
    }

    agents
}

/// Parse a single agent YAML file.
fn parse_agent_yaml(content: &str) -> Result<AgentDefinition, String> {
    let mut map = std::collections::BTreeMap::<String, serde_json::Value>::new();
    let mut tools = Vec::new();
    let mut triggers = Vec::new();
    let mut in_tools = false;
    let mut in_triggers = false;
    let mut in_system_prompt = false;
    let mut system_prompt_lines = Vec::new();
    let mut current_trigger: Option<std::collections::BTreeMap<String, serde_json::Value>> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if in_system_prompt { system_prompt_lines.push(""); }
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        // End multi-line system_prompt when we hit a non-indented key
        if in_system_prompt && indent == 0 && trimmed.contains(':') {
            in_system_prompt = false;
            map.insert("system_prompt".to_string(),
                serde_json::json!(system_prompt_lines.join("\n").trim()));
            system_prompt_lines.clear();
        }

        if in_system_prompt {
            system_prompt_lines.push(trimmed);
            continue;
        }

        if trimmed == "tools:" {
            in_tools = true;
            in_triggers = false;
            continue;
        }
        if trimmed == "triggers:" {
            in_triggers = true;
            in_tools = false;
            continue;
        }

        if in_tools && trimmed.starts_with("- ") {
            let tool = trimmed.strip_prefix("- ").unwrap().trim().trim_matches('"');
            tools.push(tool.to_string());
            continue;
        } else if in_tools && indent == 0 {
            in_tools = false;
        }

        if in_triggers {
            if trimmed.starts_with("- type:") {
                if let Some(t) = current_trigger.take() {
                    if let Ok(td) = serde_json::from_value::<TriggerDef>(serde_json::Value::Object(t.into_iter().collect())) {
                        triggers.push(td);
                    }
                }
                let val = trimmed.strip_prefix("- type:").unwrap().trim().trim_matches('"');
                let mut t = std::collections::BTreeMap::new();
                t.insert("type".to_string(), serde_json::json!(val));
                current_trigger = Some(t);
                continue;
            }
            if let Some(t) = current_trigger.as_mut() {
                if let Some((k, v)) = trimmed.split_once(':') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    if let Ok(n) = v.parse::<u64>() {
                        t.insert(k.to_string(), serde_json::json!(n));
                    } else {
                        t.insert(k.to_string(), serde_json::json!(v));
                    }
                }
                continue;
            }
            if indent == 0 { in_triggers = false; }
        }

        // Regular key: value
        if let Some((key, val)) = trimmed.split_once(':') {
            let key = key.trim();
            let val = val.trim();

            // Check for multi-line indicator
            if (val == "|" || val == ">")
                && key == "system_prompt" {
                    in_system_prompt = true;
                    continue;
                }

            let val = val.trim_matches('"').trim_matches('\'');
            if !key.is_empty() && !val.is_empty() {
                if val == "true" { map.insert(key.to_string(), serde_json::json!(true)); }
                else if val == "false" { map.insert(key.to_string(), serde_json::json!(false)); }
                else if let Ok(n) = val.parse::<u64>() { map.insert(key.to_string(), serde_json::json!(n)); }
                else { map.insert(key.to_string(), serde_json::json!(val)); }
            }
        }
    }

    // Finalize
    if in_system_prompt {
        map.insert("system_prompt".to_string(),
            serde_json::json!(system_prompt_lines.join("\n").trim()));
    }
    if let Some(t) = current_trigger.take() {
        if let Ok(td) = serde_json::from_value::<TriggerDef>(serde_json::Value::Object(t.into_iter().collect())) {
            triggers.push(td);
        }
    }

    if !tools.is_empty() { map.insert("tools".to_string(), serde_json::json!(tools)); }
    if !triggers.is_empty() { map.insert("triggers".to_string(), serde_json::json!(triggers)); }

    serde_json::from_value::<AgentDefinition>(serde_json::Value::Object(map.into_iter().collect()))
        .map_err(|e| format!("parse error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_agent() {
        let yaml = "name: test-agent\ndescription: A test\nmodel: qwen3:8b\ntools:\n  - bash\n  - read_file\nmax_iterations: 5\n";
        let agent = parse_agent_yaml(yaml).unwrap();
        assert_eq!(agent.name, "test-agent");
        assert_eq!(agent.model, "qwen3:8b");
        assert_eq!(agent.tools.len(), 2);
        assert_eq!(agent.max_iterations, 5);
    }

    #[test]
    fn parses_multiline_system_prompt() {
        let yaml = "name: support\nsystem_prompt: |\n  You are helpful.\n  Be concise.\nmodel: gemma4:26b\ntools:\n  - bash\n";
        let agent = parse_agent_yaml(yaml).unwrap();
        assert!(agent.system_prompt.contains("You are helpful"));
        assert!(agent.system_prompt.contains("Be concise"));
    }

    #[test]
    fn parses_triggers() {
        let yaml = "name: monitor\ntriggers:\n  - type: webhook\n    path: /alerts\n  - type: schedule\n    interval_seconds: 600\ntools:\n  - bash\n";
        let agent = parse_agent_yaml(yaml).unwrap();
        assert_eq!(agent.triggers.len(), 2);
        assert_eq!(agent.triggers[0].r#type, "webhook");
        assert_eq!(agent.triggers[1].interval_seconds, Some(600));
    }

    #[test]
    fn converts_to_template() {
        let agent = AgentDefinition {
            name: "test".to_string(),
            description: "Test agent".to_string(),
            model: "gemma4:26b".to_string(),
            system_prompt: "Be helpful".to_string(),
            tools: vec!["bash".to_string()],
            max_iterations: 10,
            approval_required: false,
            use_planning: true,
            triggers: vec![],
        };
        let template = agent.to_template();
        assert_eq!(template.name, "test");
        assert_eq!(template.model, "gemma4:26b");
    }
}
