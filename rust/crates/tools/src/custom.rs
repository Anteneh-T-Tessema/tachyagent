//! Custom tool registry — users define tools in `.tachy/tools.yaml`.
//!
//! A custom tool is a shell command, HTTP endpoint, or database query
//! that Tachy executes with the same audit trail, governance, and
//! permissions as built-in tools.
//!
//! Example `.tachy/tools.yaml`:
//! ```yaml
//! tools:
//!   - name: query_db
//!     description: "Run a read-only SQL query against the app database"
//!     type: shell
//!     command: "psql $DATABASE_URL -c \"{query}\""
//!     parameters:
//!       query:
//!         type: string
//!         description: "SQL query to execute"
//!         required: true
//!     approval_required: true
//!     timeout_secs: 30
//!
//!   - name: send_slack
//!     description: "Send a message to a Slack channel"
//!     type: http
//!     method: POST
//!     url: "https://hooks.slack.com/services/T00/B00/xxx"
//!     headers:
//!       Content-Type: application/json
//!     body_template: '{"text": "{message}", "channel": "{channel}"}'
//!     parameters:
//!       message:
//!         type: string
//!         required: true
//!       channel:
//!         type: string
//!         default: "#general"
//!
//!   - name: list_tickets
//!     description: "List open support tickets"
//!     type: http
//!     method: GET
//!     url: "https://api.example.com/tickets?status=open"
//!     headers:
//!       Authorization: "Bearer $SUPPORT_API_KEY"
//! ```

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A custom tool definition loaded from YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTool {
    pub name: String,
    pub description: String,
    #[serde(default = "default_tool_type")]
    pub r#type: ToolType,
    /// Shell command template (for type: shell). Use {`param_name`} for substitution.
    #[serde(default)]
    pub command: Option<String>,
    /// HTTP method (for type: http).
    #[serde(default)]
    pub method: Option<String>,
    /// HTTP URL template (for type: http). Use {`param_name`} for substitution.
    #[serde(default)]
    pub url: Option<String>,
    /// HTTP headers (for type: http). Values can contain $`ENV_VAR` references.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// HTTP body template (for type: http). Use {`param_name`} for substitution.
    #[serde(default)]
    pub body_template: Option<String>,
    /// Parameter definitions.
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamDef>,
    /// Whether this tool requires human approval before execution.
    #[serde(default)]
    pub approval_required: bool,
    /// Timeout in seconds (default: 30).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Tags for categorization.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    Shell,
    Http,
}

fn default_tool_type() -> ToolType { ToolType::Shell }
fn default_timeout() -> u64 { 30 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    #[serde(default = "default_param_type")]
    pub r#type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

fn default_param_type() -> String { "string".to_string() }

/// The tools.yaml file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolsFile {
    #[serde(default)]
    pub tools: Vec<CustomTool>,
}

/// Registry of custom tools loaded from disk.
#[derive(Debug, Clone, Default)]
pub struct CustomToolRegistry {
    tools: Vec<CustomTool>,
}

impl CustomToolRegistry {
    /// Load custom tools from `.tachy/tools.yaml`.
    #[must_use] pub fn load(tachy_dir: &Path) -> Self {
        let yaml_path = tachy_dir.join("tools.yaml");
        let yml_path = tachy_dir.join("tools.yml");

        let path = if yaml_path.exists() {
            yaml_path
        } else if yml_path.exists() {
            yml_path
        } else {
            return Self::default();
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        // Parse YAML manually (no serde_yaml dependency — use simple line parser)
        match parse_tools_yaml(&content) {
            Ok(tools) => Self { tools },
            Err(e) => {
                eprintln!("warning: failed to parse {}: {e}", path.display());
                Self::default()
            }
        }
    }

    #[must_use] pub fn tools(&self) -> &[CustomTool] {
        &self.tools
    }

    #[must_use] pub fn find(&self, name: &str) -> Option<&CustomTool> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// Generate tool specs for the LLM (same format as built-in tools).
    #[must_use] pub fn tool_specs(&self) -> Vec<super::ToolSpec> {
        self.tools.iter().map(|t| {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();

            for (name, param) in &t.parameters {
                let mut prop = serde_json::Map::new();
                prop.insert("type".to_string(), json!(param.r#type));
                if let Some(desc) = &param.description {
                    prop.insert("description".to_string(), json!(desc));
                }
                properties.insert(name.clone(), Value::Object(prop));
                if param.required {
                    required.push(json!(name));
                }
            }

            // Leak the strings so we get &'static str (safe for program lifetime)
            let name: &'static str = Box::leak(t.name.clone().into_boxed_str());
            let desc: &'static str = Box::leak(t.description.clone().into_boxed_str());

            super::ToolSpec {
                name,
                description: desc,
                input_schema: json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }),
            }
        }).collect()
    }

    /// Execute a custom tool with the given input.
    pub fn execute(&self, name: &str, input: &Value) -> Result<String, String> {
        let tool = self.find(name).ok_or_else(|| format!("custom tool not found: {name}"))?;

        match tool.r#type {
            ToolType::Shell => execute_shell_tool(tool, input),
            ToolType::Http => execute_http_tool(tool, input),
        }
    }
}

/// Execute a shell-type custom tool.
fn execute_shell_tool(tool: &CustomTool, input: &Value) -> Result<String, String> {
    let template = tool.command.as_deref()
        .ok_or_else(|| format!("tool '{}' has type=shell but no command", tool.name))?;

    // Substitute parameters into the command template
    let command = substitute_params(template, &tool.parameters, input)?;

    // Sanitize: block dangerous patterns
    let lower = command.to_lowercase();
    for pattern in ["rm -rf /", "mkfs", "dd if=", "> /dev/"] {
        if lower.contains(pattern) {
            return Err(format!("blocked: command contains dangerous pattern '{pattern}'"));
        }
    }

    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let flag = if cfg!(target_os = "windows") { "/C" } else { "-c" };

    let timeout_secs: u64 = std::env::var("TACHY_TOOL_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
        .max(5)
        .min(300);

    // Run in a background thread so we can enforce a hard deadline.
    let command_owned = command.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new(shell)
            .arg(flag)
            .arg(&command_owned)
            .output();
        let _ = tx.send(result);
    });
    let output = rx
        .recv_timeout(std::time::Duration::from_secs(timeout_secs))
        .map_err(|_| format!("custom tool timed out after {timeout_secs}s"))?
        .map_err(|e| format!("failed to execute command: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        if stdout.is_empty() && !stderr.is_empty() {
            Ok(stderr)
        } else {
            Ok(stdout)
        }
    } else {
        Err(format!("command failed (exit {}): {}", output.status.code().unwrap_or(-1), stderr))
    }
}

/// Execute an HTTP-type custom tool.
fn execute_http_tool(tool: &CustomTool, input: &Value) -> Result<String, String> {
    let url_template = tool.url.as_deref()
        .ok_or_else(|| format!("tool '{}' has type=http but no url", tool.name))?;
    let method = tool.method.as_deref().unwrap_or("GET").to_uppercase();

    let url = substitute_params(url_template, &tool.parameters, input)?;

    // Build the request using curl (no additional Rust HTTP dependency needed)
    let mut args = vec!["-s".to_string(), "-S".to_string(), "--max-time".to_string(), tool.timeout_secs.to_string()];

    args.push("-X".to_string());
    args.push(method);

    // Add headers (expand env vars)
    for (key, value) in &tool.headers {
        let expanded = expand_env_vars(value);
        args.push("-H".to_string());
        args.push(format!("{key}: {expanded}"));
    }

    // Add body if present
    if let Some(body_template) = &tool.body_template {
        let body = substitute_params(body_template, &tool.parameters, input)?;
        args.push("-d".to_string());
        args.push(body);
    }

    args.push(url);

    let output = Command::new("curl")
        .args(&args)
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("HTTP request failed: {stderr}"))
    }
}

/// Substitute {`param_name`} placeholders in a template string.
fn substitute_params(
    template: &str,
    param_defs: &BTreeMap<String, ParamDef>,
    input: &Value,
) -> Result<String, String> {
    let mut result = template.to_string();
    let obj = input.as_object();

    for (name, def) in param_defs {
        let placeholder = format!("{{{name}}}");
        if result.contains(&placeholder) {
            let value = obj
                .and_then(|o| o.get(name))
                .and_then(|v| v.as_str().map(std::string::ToString::to_string).or_else(|| Some(v.to_string())))
                .or_else(|| def.default.clone())
                .ok_or_else(|| {
                    if def.required {
                        format!("required parameter '{name}' not provided")
                    } else {
                        format!("parameter '{name}' not provided and has no default")
                    }
                })?;

            // Sanitize the value to prevent injection
            let sanitized = sanitize_param_value(&value);
            result = result.replace(&placeholder, &sanitized);
        }
    }

    // Expand environment variables ($VAR_NAME)
    result = expand_env_vars(&result);

    Ok(result)
}

/// Sanitize a parameter value to prevent shell injection.
fn sanitize_param_value(value: &str) -> String {
    // Remove shell metacharacters that could cause injection
    value
        .replace('`', "")
        .replace("$(", "")
        .replace("${", "")
        .replace([';', '|', '&'], "")
        .replace('\n', " ")
        .replace('\r', "")
}

/// Expand $`ENV_VAR` references in a string.
fn expand_env_vars(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut output = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_') {
            // Read the variable name
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let var_name: String = chars[start..end].iter().collect();
            if let Ok(val) = std::env::var(&var_name) {
                output.push_str(&val);
            } else {
                // Keep the original $VAR if not found
                output.push('$');
                output.push_str(&var_name);
            }
            i = end;
        } else {
            output.push(chars[i]);
            i += 1;
        }
    }
    output
}

/// Simple YAML parser for tools.yaml (avoids `serde_yaml` dependency).
/// Handles the specific structure we need: a list of tool objects.
fn parse_tools_yaml(content: &str) -> Result<Vec<CustomTool>, String> {
    // Use serde_json by converting our simple YAML to JSON
    // This handles the 90% case without adding a YAML crate
    let mut tools = Vec::new();
    let mut current_tool: Option<serde_json::Map<String, Value>> = None;
    let mut current_params: Option<serde_json::Map<String, Value>> = None;
    let mut current_headers: Option<serde_json::Map<String, Value>> = None;
    let mut current_tags: Option<Vec<Value>> = None;
    let mut current_param_name: Option<String> = None;
    let mut current_param: Option<serde_json::Map<String, Value>> = None;
    let mut in_params = false;
    let mut in_headers = false;
    let mut in_tags = false;
    let mut in_param_detail = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        // Top level: "tools:"
        if trimmed == "tools:" {
            continue;
        }

        // New tool: "  - name: xxx"
        if trimmed.starts_with("- name:") {
            // Save previous tool
            if let Some(mut tool) = current_tool.take() {
                if let Some(params) = current_params.take() {
                    tool.insert("parameters".to_string(), Value::Object(params));
                }
                if let Some(headers) = current_headers.take() {
                    tool.insert("headers".to_string(), Value::Object(headers));
                }
                if let Some(tags) = current_tags.take() {
                    tool.insert("tags".to_string(), Value::Array(tags));
                }
                if let Ok(t) = serde_json::from_value::<CustomTool>(Value::Object(tool)) {
                    tools.push(t);
                }
            }
            let name = trimmed.strip_prefix("- name:").unwrap().trim().trim_matches('"').trim_matches('\'');
            let mut map = serde_json::Map::new();
            map.insert("name".to_string(), json!(name));
            current_tool = Some(map);
            current_params = None;
            current_headers = None;
            current_tags = None;
            in_params = false;
            in_headers = false;
            in_tags = false;
            continue;
        }

        if let Some(tool) = current_tool.as_mut() {
            // Check for section starts
            if trimmed == "parameters:" {
                in_params = true;
                in_headers = false;
                in_tags = false;
                in_param_detail = false;
                current_params = Some(serde_json::Map::new());
                continue;
            }
            if trimmed == "headers:" {
                in_headers = true;
                in_params = false;
                in_tags = false;
                current_headers = Some(serde_json::Map::new());
                continue;
            }
            if trimmed == "tags:" {
                in_tags = true;
                in_params = false;
                in_headers = false;
                current_tags = Some(Vec::new());
                continue;
            }

            if in_tags {
                if trimmed.starts_with("- ") {
                    let val = trimmed.strip_prefix("- ").unwrap().trim().trim_matches('"').trim_matches('\'');
                    if let Some(tags) = current_tags.as_mut() {
                        tags.push(json!(val));
                    }
                    continue;
                } else if !trimmed.starts_with('-') && indent <= 4 {
                    in_tags = false;
                }
            }

            if in_headers {
                if let Some((key, val)) = trimmed.split_once(':') {
                    let key = key.trim();
                    let val = val.trim().trim_matches('"').trim_matches('\'');
                    if !key.is_empty() && indent >= 6 {
                        if let Some(headers) = current_headers.as_mut() {
                            headers.insert(key.to_string(), json!(val));
                        }
                        continue;
                    }
                }
                if indent <= 4 { in_headers = false; }
            }

            if in_params {
                // Parameter name line: "      query:" (indent 6+, ends with :, no value)
                if indent >= 6 && trimmed.ends_with(':') && !trimmed.contains(": ") {
                    // Save previous param
                    if let (Some(pname), Some(pmap)) = (current_param_name.take(), current_param.take()) {
                        if let Some(params) = current_params.as_mut() {
                            params.insert(pname, Value::Object(pmap));
                        }
                    }
                    let pname = trimmed.trim_end_matches(':').trim();
                    current_param_name = Some(pname.to_string());
                    current_param = Some(serde_json::Map::new());
                    in_param_detail = true;
                    continue;
                }
                // Parameter detail: "        type: string"
                if in_param_detail && indent >= 8 {
                    if let Some((key, val)) = trimmed.split_once(':') {
                        let key = key.trim();
                        let val = val.trim().trim_matches('"').trim_matches('\'');
                        if let Some(param) = current_param.as_mut() {
                            if val == "true" {
                                param.insert(key.to_string(), json!(true));
                            } else if val == "false" {
                                param.insert(key.to_string(), json!(false));
                            } else {
                                param.insert(key.to_string(), json!(val));
                            }
                        }
                    }
                    continue;
                }
                if indent <= 4 {
                    in_params = false;
                    in_param_detail = false;
                    // Save last param
                    if let (Some(pname), Some(pmap)) = (current_param_name.take(), current_param.take()) {
                        if let Some(params) = current_params.as_mut() {
                            params.insert(pname, Value::Object(pmap));
                        }
                    }
                }
            }

            // Regular key: value on the tool
            if !in_params && !in_headers && !in_tags {
                if let Some((key, val)) = trimmed.split_once(':') {
                    let key = key.trim().trim_start_matches("- ");
                    let val = val.trim().trim_matches('"').trim_matches('\'');
                    if !key.is_empty() && !val.is_empty() {
                        if val == "true" {
                            tool.insert(key.to_string(), json!(true));
                        } else if val == "false" {
                            tool.insert(key.to_string(), json!(false));
                        } else if let Ok(n) = val.parse::<u64>() {
                            tool.insert(key.to_string(), json!(n));
                        } else {
                            tool.insert(key.to_string(), json!(val));
                        }
                    }
                }
            }
        }
    }

    // Save last tool
    if let Some(mut tool) = current_tool.take() {
        // Save last param
        if let (Some(pname), Some(pmap)) = (current_param_name.take(), current_param.take()) {
            if let Some(params) = current_params.as_mut() {
                params.insert(pname, Value::Object(pmap));
            }
        }
        if let Some(params) = current_params.take() {
            tool.insert("parameters".to_string(), Value::Object(params));
        }
        if let Some(headers) = current_headers.take() {
            tool.insert("headers".to_string(), Value::Object(headers));
        }
        if let Some(tags) = current_tags.take() {
            tool.insert("tags".to_string(), Value::Array(tags));
        }
        if let Ok(t) = serde_json::from_value::<CustomTool>(Value::Object(tool)) {
            tools.push(t);
        }
    }

    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shell_tool() {
        let yaml = r#"
tools:
  - name: list_users
    description: List all users in the database
    type: shell
    command: "psql $DB_URL -c \"{query}\""
    parameters:
      query:
        type: string
        description: SQL query
        required: true
    timeout_secs: 15
"#;
        let tools = parse_tools_yaml(yaml).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "list_users");
        assert_eq!(tools[0].r#type, ToolType::Shell);
        assert!(tools[0].parameters.contains_key("query"));
        assert!(tools[0].parameters["query"].required);
        assert_eq!(tools[0].timeout_secs, 15);
    }

    #[test]
    fn parses_http_tool() {
        let yaml = r#"
tools:
  - name: send_slack
    description: Send a Slack message
    type: http
    method: POST
    url: "https://hooks.slack.com/services/xxx"
    headers:
      Content-Type: application/json
    body_template: '{"text": "{message}"}'
    parameters:
      message:
        type: string
        required: true
"#;
        let tools = parse_tools_yaml(yaml).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "send_slack");
        assert_eq!(tools[0].r#type, ToolType::Http);
        assert_eq!(tools[0].method.as_deref(), Some("POST"));
        assert!(tools[0].headers.contains_key("Content-Type"));
    }

    #[test]
    fn parses_multiple_tools() {
        let yaml = r#"
tools:
  - name: tool_a
    description: First tool
    type: shell
    command: "echo a"
  - name: tool_b
    description: Second tool
    type: shell
    command: "echo b"
"#;
        let tools = parse_tools_yaml(yaml).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "tool_a");
        assert_eq!(tools[1].name, "tool_b");
    }

    #[test]
    fn substitutes_params() {
        let mut params = BTreeMap::new();
        params.insert("name".to_string(), ParamDef {
            r#type: "string".to_string(),
            description: None,
            required: true,
            default: None,
        });
        let input = json!({"name": "Alice"});
        let result = substitute_params("hello {name}", &params, &input).unwrap();
        assert_eq!(result, "hello Alice");
    }

    #[test]
    fn sanitizes_injection() {
        let sanitized = sanitize_param_value("test; rm -rf /");
        assert!(!sanitized.contains(';'));
    }

    #[test]
    fn generates_tool_specs() {
        let yaml = r#"
tools:
  - name: my_tool
    description: A custom tool
    type: shell
    command: "echo {msg}"
    parameters:
      msg:
        type: string
        required: true
"#;
        let tools = parse_tools_yaml(yaml).unwrap();
        let registry = CustomToolRegistry { tools };
        let specs = registry.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "my_tool");
        assert!(specs[0].input_schema.to_string().contains("msg"));
    }

    #[test]
    fn executes_shell_tool() {
        let tool = CustomTool {
            name: "echo_test".to_string(),
            description: "test".to_string(),
            r#type: ToolType::Shell,
            command: Some("echo {msg}".to_string()),
            method: None, url: None, headers: BTreeMap::new(),
            body_template: None,
            parameters: {
                let mut p = BTreeMap::new();
                p.insert("msg".to_string(), ParamDef {
                    r#type: "string".to_string(),
                    description: None,
                    required: true,
                    default: None,
                });
                p
            },
            approval_required: false,
            timeout_secs: 10,
            tags: vec![],
        };
        let result = execute_shell_tool(&tool, &json!({"msg": "hello"}));
        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[test]
    fn blocks_dangerous_commands() {
        let tool = CustomTool {
            name: "bad".to_string(),
            description: "test".to_string(),
            r#type: ToolType::Shell,
            command: Some("rm -rf / {path}".to_string()),
            method: None, url: None, headers: BTreeMap::new(),
            body_template: None,
            parameters: {
                let mut p = BTreeMap::new();
                p.insert("path".to_string(), ParamDef {
                    r#type: "string".to_string(),
                    description: None,
                    required: true,
                    default: None,
                });
                p
            },
            approval_required: false,
            timeout_secs: 10,
            tags: vec![],
        };
        let result = execute_shell_tool(&tool, &json!({"path": "/tmp"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked"));
    }
}
