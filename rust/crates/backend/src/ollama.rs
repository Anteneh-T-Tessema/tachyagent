use std::time::Duration;

use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, MessageRole, RuntimeError, TokenUsage,
};
use serde::{Deserialize, Serialize};
use tools::mvp_tool_specs;

/// Maximum size of tool output to feed back to the model (bytes).
/// Local models with small context windows choke on huge outputs.
const MAX_TOOL_OUTPUT_CHARS: usize = 8_000;

/// Maximum retries for transient Ollama failures.
const MAX_RETRIES: u32 = 2;

/// HTTP request timeout for Ollama calls.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Connection timeout for Ollama.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct OllamaBackend {
    client: reqwest::Client,
    base_url: String,
    model: String,
    enable_tools: bool,
}

impl OllamaBackend {
    pub fn new(
        model: String,
        base_url: String,
        enable_tools: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            client,
            base_url,
            model,
            enable_tools,
        })
    }

    /// Check if Ollama is reachable.
    pub fn health_check(&self) -> Result<(), String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(async {
            self.client
                .get(format!("{}/api/tags", self.base_url))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|e| format!("ollama not reachable at {}: {e}", self.base_url))?;
            Ok(())
        })
    }
}

impl ApiClient for OllamaBackend {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let messages = convert_to_ollama_messages(&request);
        let tools = if self.enable_tools {
            Some(build_ollama_tools())
        } else {
            None
        };

        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages,
            stream: false, // TODO: switch to true for real-time streaming when wired to SSE
            tools,
            options: Some(OllamaOptions {
                num_ctx: Some(8192),
                temperature: Some(0.1),
                num_predict: Some(4096),
            }),
        };

        let future = self.send_with_retry(body);

        if tokio::runtime::Handle::try_current().is_ok() {
            std::thread::scope(|s| {
                s.spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| RuntimeError::new(e.to_string()))?;
                    rt.block_on(future)
                })
                .join()
                .map_err(|_| RuntimeError::new("ollama request thread panicked"))?
            })
        } else {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            rt.block_on(future)
        }
    }
}

impl OllamaBackend {
    async fn send_with_retry(
        &self,
        body: OllamaChatRequest,
    ) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let mut last_error = RuntimeError::new("no attempts made");

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
            }

            match self.send_request(&body).await {
                Ok(events) => return Ok(events),
                Err(e) => {
                    let msg = e.to_string();
                    // Don't retry on 404 (model not found) or 400 (bad request)
                    if msg.contains("404") || msg.contains("400") {
                        return Err(e);
                    }
                    last_error = e;
                }
            }
        }

        Err(RuntimeError::new(format!(
            "ollama failed after {} attempts: {last_error}",
            MAX_RETRIES + 1
        )))
    }

    async fn send_request(
        &self,
        body: &OllamaChatRequest,
    ) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    RuntimeError::new(format!(
                        "cannot connect to ollama at {} — is it running? ({})",
                        self.base_url, e
                    ))
                } else if e.is_timeout() {
                    RuntimeError::new(format!(
                        "ollama request timed out after {}s — model may be loading or too slow",
                        REQUEST_TIMEOUT.as_secs()
                    ))
                } else {
                    RuntimeError::new(format!("ollama request failed: {e}"))
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();

            // Parse Ollama error for better messages
            if let Ok(err) = serde_json::from_str::<OllamaErrorResponse>(&body_text) {
                if status.as_u16() == 404 {
                    return Err(RuntimeError::new(format!(
                        "model '{}' not found in ollama — run `ollama pull {}` first",
                        self.model, self.model
                    )));
                }
                return Err(RuntimeError::new(format!(
                    "ollama error ({}): {}",
                    status, err.error
                )));
            }

            return Err(RuntimeError::new(format!(
                "ollama returned {status}: {body_text}"
            )));
        }

        let chat_response: OllamaChatResponse = response.json().await.map_err(|e| {
            RuntimeError::new(format!("ollama response parse error: {e}"))
        })?;

        parse_ollama_response(chat_response)
    }
}

fn parse_ollama_response(
    response: OllamaChatResponse,
) -> Result<Vec<AssistantEvent>, RuntimeError> {
    let mut events = Vec::new();
    let message = response.message;

    if let Some(tool_calls) = &message.tool_calls {
        if !tool_calls.is_empty() {
            // Emit text before tool calls if present
            if !message.content.is_empty() {
                events.push(AssistantEvent::TextDelta(message.content.clone()));
            }

            for (i, call) in tool_calls.iter().enumerate() {
                let input = serde_json::to_string(&call.function.arguments)
                    .unwrap_or_else(|_| "{}".to_string());
                let id = call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("ollama-tool-{i}"));

                // Validate tool call has a non-empty name
                if call.function.name.is_empty() {
                    continue;
                }

                events.push(AssistantEvent::ToolUse {
                    id,
                    name: call.function.name.clone(),
                    input,
                });
            }
        }
    }

    // If no tool calls were emitted, use the text content
    if !events.iter().any(|e| matches!(e, AssistantEvent::ToolUse { .. }))
        && !message.content.is_empty()
    {
        // Check if the model printed a tool call as text (common with local models)
        if let Some(repaired) = try_repair_tool_call_from_text(&message.content) {
            events.push(repaired);
        } else {
            events.push(AssistantEvent::TextDelta(message.content));
        }
    }

    // Usage tracking
    let usage = TokenUsage {
        input_tokens: response.prompt_eval_count.unwrap_or(0),
        output_tokens: response.eval_count.unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    };
    events.push(AssistantEvent::Usage(usage));
    events.push(AssistantEvent::MessageStop);

    // Must have at least one content event beyond usage+stop
    if !events
        .iter()
        .any(|e| matches!(e, AssistantEvent::TextDelta(_) | AssistantEvent::ToolUse { .. }))
    {
        return Err(RuntimeError::new(
            "ollama returned empty response — model may not support this request format",
        ));
    }

    Ok(events)
}

/// Local models sometimes print tool calls as JSON text instead of using the
/// tool_calls field. Try to extract a valid tool call from the text.
fn try_repair_tool_call_from_text(text: &str) -> Option<AssistantEvent> {
    // Look for JSON-like patterns: {"name": "tool_name", ...}
    let trimmed = text.trim();

    // Try to find a JSON object in the text
    let json_start = trimmed.find('{')?;
    let json_end = trimmed.rfind('}')?;
    if json_end <= json_start {
        return None;
    }

    let candidate = &trimmed[json_start..=json_end];
    let parsed: serde_json::Value = serde_json::from_str(candidate).ok()?;
    let obj = parsed.as_object()?;

    // Pattern 1: {"name": "bash", "parameters": {"command": "ls"}}
    if let (Some(name), Some(params)) = (
        obj.get("name").and_then(|v| v.as_str()),
        obj.get("parameters"),
    ) {
        let known_tools = ["bash", "read_file", "write_file", "edit_file", "glob_search", "grep_search"];
        if known_tools.contains(&name) {
            return Some(AssistantEvent::ToolUse {
                id: "repaired-0".to_string(),
                name: name.to_string(),
                input: serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string()),
            });
        }
    }

    // Pattern 2: {"command": "ls"} — assume bash if it has a "command" field
    if obj.contains_key("command") && !obj.contains_key("name") {
        return Some(AssistantEvent::ToolUse {
            id: "repaired-0".to_string(),
            name: "bash".to_string(),
            input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
        });
    }

    // Pattern 3: {"path": "..."} — assume read_file
    if obj.contains_key("path") && obj.len() <= 3 && !obj.contains_key("name") {
        return Some(AssistantEvent::ToolUse {
            id: "repaired-0".to_string(),
            name: "read_file".to_string(),
            input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
        });
    }

    None
}

/// Truncate tool output to prevent context window overflow with local models.
pub fn truncate_tool_output(output: &str) -> String {
    if output.len() <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }
    let truncated = &output[..MAX_TOOL_OUTPUT_CHARS];
    format!(
        "{truncated}\n\n[output truncated — showing first {MAX_TOOL_OUTPUT_CHARS} of {} chars]",
        output.len()
    )
}

fn convert_to_ollama_messages(request: &ApiRequest) -> Vec<OllamaMessage> {
    let mut messages = Vec::new();

    if !request.system_prompt.is_empty() {
        messages.push(OllamaMessage {
            role: "system".to_string(),
            content: request.system_prompt.join("\n\n"),
            tool_calls: None,
        });
    }

    for msg in &request.messages {
        match msg.role {
            MessageRole::System => {
                let text = extract_text_content(&msg.blocks);
                if !text.is_empty() {
                    messages.push(OllamaMessage {
                        role: "system".to_string(),
                        content: text,
                        tool_calls: None,
                    });
                }
            }
            MessageRole::User => {
                let text = extract_text_content(&msg.blocks);
                if !text.is_empty() {
                    messages.push(OllamaMessage {
                        role: "user".to_string(),
                        content: text,
                        tool_calls: None,
                    });
                }
            }
            MessageRole::Assistant => {
                let text = extract_text_content(&msg.blocks);
                let tool_calls = extract_tool_calls(&msg.blocks);
                messages.push(OllamaMessage {
                    role: "assistant".to_string(),
                    content: text,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                });
            }
            MessageRole::Tool => {
                // Truncate tool results to prevent context overflow
                let text = truncate_tool_output(&extract_tool_result_content(&msg.blocks));
                messages.push(OllamaMessage {
                    role: "tool".to_string(),
                    content: text,
                    tool_calls: None,
                });
            }
        }
    }

    messages
}

fn extract_text_content(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_tool_calls(blocks: &[ContentBlock]) -> Vec<OllamaToolCall> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => {
                let arguments: serde_json::Value = serde_json::from_str(input)
                    .unwrap_or_else(|_| serde_json::json!({ "raw": input }));
                Some(OllamaToolCall {
                    id: Some(id.clone()),
                    function: OllamaFunctionCall {
                        name: name.clone(),
                        arguments,
                        index: None,
                    },
                })
            }
            _ => None,
        })
        .collect()
}

fn extract_tool_result_content(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult { output, .. } => Some(output.as_str()),
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_ollama_tools() -> Vec<OllamaTool> {
    mvp_tool_specs()
        .into_iter()
        .map(|spec| OllamaTool {
            r#type: "function".to_string(),
            function: OllamaToolFunction {
                name: spec.name.to_string(),
                description: spec.description.to_string(),
                parameters: spec.input_schema,
            },
        })
        .collect()
}

// --- Ollama API types ---

#[derive(Debug, Clone, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaToolCall {
    #[serde(default)]
    id: Option<String>,
    function: OllamaFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: serde_json::Value,
    #[serde(default)]
    index: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaTool {
    r#type: String,
    function: OllamaToolFunction,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaErrorResponse {
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::ConversationMessage;

    #[test]
    fn converts_system_and_user_messages() {
        let request = ApiRequest {
            system_prompt: vec!["You are helpful.".to_string()],
            messages: vec![ConversationMessage::user_text("hello")],
        };
        let messages = convert_to_ollama_messages(&request);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "hello");
    }

    #[test]
    fn builds_tool_definitions() {
        let tools = build_ollama_tools();
        let names: Vec<_> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"grep_search"));
    }

    #[test]
    fn truncates_large_tool_output() {
        let small = "hello world";
        assert_eq!(truncate_tool_output(small), small);

        let large = "x".repeat(10_000);
        let truncated = truncate_tool_output(&large);
        assert!(truncated.len() < large.len());
        assert!(truncated.contains("[output truncated"));
        assert!(truncated.contains("10000 chars"));
    }

    #[test]
    fn repairs_tool_call_from_text_pattern1() {
        let text = r#"I'll read the file. {"name": "read_file", "parameters": {"path": "Cargo.toml"}}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "read_file");
                assert!(input.contains("Cargo.toml"));
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn repairs_tool_call_from_text_bash_shorthand() {
        let text = r#"{"command": "ls -la"}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "bash");
                assert!(input.contains("ls -la"));
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn repairs_tool_call_from_text_path_shorthand() {
        let text = r#"{"path": "src/main.rs"}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "read_file");
                assert!(input.contains("src/main.rs"));
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn does_not_repair_normal_text() {
        let text = "The answer is 42. Here's what I found in the codebase.";
        assert!(try_repair_tool_call_from_text(text).is_none());
    }

    #[test]
    fn does_not_repair_unknown_tool_json() {
        let text = r#"{"name": "unknown_tool", "parameters": {}}"#;
        assert!(try_repair_tool_call_from_text(text).is_none());
    }

    #[test]
    fn parses_response_with_tool_calls() {
        let response = OllamaChatResponse {
            message: OllamaResponseMessage {
                content: String::new(),
                tool_calls: Some(vec![OllamaToolCall {
                    id: Some("call-1".to_string()),
                    function: OllamaFunctionCall {
                        name: "bash".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                        index: Some(0),
                    },
                }]),
            },
            prompt_eval_count: Some(100),
            eval_count: Some(20),
        };

        let events = parse_ollama_response(response).expect("should parse");
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::ToolUse { name, .. } if name == "bash")));
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::Usage(u) if u.input_tokens == 100)));
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::MessageStop)));
    }

    #[test]
    fn parses_response_with_text_only() {
        let response = OllamaChatResponse {
            message: OllamaResponseMessage {
                content: "Hello world".to_string(),
                tool_calls: None,
            },
            prompt_eval_count: Some(10),
            eval_count: Some(5),
        };

        let events = parse_ollama_response(response).expect("should parse");
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::TextDelta(t) if t == "Hello world")));
    }

    #[test]
    fn rejects_empty_response() {
        let response = OllamaChatResponse {
            message: OllamaResponseMessage {
                content: String::new(),
                tool_calls: None,
            },
            prompt_eval_count: None,
            eval_count: None,
        };

        let result = parse_ollama_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty response"));
    }

    #[test]
    fn skips_tool_calls_with_empty_name() {
        let response = OllamaChatResponse {
            message: OllamaResponseMessage {
                content: "fallback text".to_string(),
                tool_calls: Some(vec![OllamaToolCall {
                    id: None,
                    function: OllamaFunctionCall {
                        name: String::new(),
                        arguments: serde_json::json!({}),
                        index: None,
                    },
                }]),
            },
            prompt_eval_count: Some(10),
            eval_count: Some(5),
        };

        let events = parse_ollama_response(response).expect("should parse");
        // Should fall back to text since the tool call had empty name
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::TextDelta(t) if t == "fallback text")));
        assert!(!events.iter().any(|e| matches!(e, AssistantEvent::ToolUse { .. })));
    }

    #[test]
    fn tool_output_truncation_in_messages() {
        let large_output = "x".repeat(10_000);
        let blocks = vec![ContentBlock::ToolResult {
            tool_use_id: "1".to_string(),
            tool_name: "bash".to_string(),
            output: large_output,
            is_error: false,
        }];
        let request = ApiRequest {
            system_prompt: vec![],
            messages: vec![runtime::ConversationMessage {
                role: MessageRole::Tool,
                blocks,
                usage: None,
            }],
        };
        let messages = convert_to_ollama_messages(&request);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.len() < 9_000);
        assert!(messages[0].content.contains("[output truncated"));
    }
}
