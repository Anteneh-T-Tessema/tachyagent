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
/// 3 retries with backoff handles cold starts where the model is loading into GPU.
const MAX_RETRIES: u32 = 3;

/// HTTP request timeout for Ollama calls.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Connection timeout for Ollama.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct OllamaBackend {
    client: reqwest::Client,
    base_url: String,
    model: String,
    enable_tools: bool,
    /// Model-specific options derived from model name.
    model_options: ModelOptions,
    /// Optional callback invoked for each text token during streaming.
    /// If set, Ollama uses `stream: true` and calls this for real-time output.
    stream_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

/// Model-specific tuning parameters.
struct ModelOptions {
    num_ctx: u32,
    temperature: f32,
    top_p: Option<f32>,
    top_k: Option<u32>,
    num_predict: u32,
}

impl ModelOptions {
    /// Pick optimal options based on model name and available system RAM.
    fn for_model(model: &str) -> Self {
        let lower = model.to_lowercase();
        let available_ram_gb = detect_available_ram_gb();

        if lower.contains("gemma4") {
            // Scale context based on available RAM
            let num_ctx = if available_ram_gb >= 48 {
                32_768  // Plenty of RAM — full context
            } else if available_ram_gb >= 24 {
                16_384  // Moderate RAM
            } else {
                8192    // Low RAM — conservative
            };
            Self {
                num_ctx,
                temperature: 1.0,
                top_p: Some(0.95),
                top_k: Some(64),
                num_predict: if available_ram_gb >= 32 { 8192 } else { 4096 },
            }
        } else if lower.contains("qwen3") {
            Self {
                num_ctx: if available_ram_gb >= 24 { 16_384 } else { 8192 },
                temperature: 0.7,
                top_p: Some(0.8),
                top_k: None,
                num_predict: 4096,
            }
        } else if lower.contains("llama3.1") && (lower.contains("70b") || lower.contains("405b")) {
            Self {
                num_ctx: if available_ram_gb >= 48 { 16_384 } else { 8192 },
                temperature: 0.6,
                top_p: None,
                top_k: None,
                num_predict: 4096,
            }
        } else {
            Self {
                num_ctx: if available_ram_gb >= 16 { 8192 } else { 4096 },
                temperature: 0.1,
                top_p: None,
                top_k: None,
                num_predict: 4096,
            }
        }
    }
}

/// Detect available system RAM in GB. Returns a conservative estimate.
fn detect_available_ram_gb() -> u64 {
    // macOS
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sysctl").arg("-n").arg("hw.memsize").output() {
            if let Ok(bytes) = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>() {
                return bytes / 1_073_741_824;
            }
        }
    }
    // Linux
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb / 1_048_576;
                        }
                    }
                }
            }
        }
    }
    // Windows
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["computersystem", "get", "TotalPhysicalMemory"])
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Ok(bytes) = line.trim().parse::<u64>() {
                    return bytes / 1_073_741_824;
                }
            }
        }
    }
    // Default: assume 16GB if we can't detect
    16
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
        let model_options = ModelOptions::for_model(&model);
        Ok(Self {
            client,
            base_url,
            model,
            enable_tools,
            model_options,
            stream_callback: None,
        })
    }

    /// Set a callback for real-time token streaming.
    pub fn set_stream_callback(&mut self, callback: impl Fn(&str) + Send + Sync + 'static) {
        self.stream_callback = Some(Box::new(callback));
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

        let use_streaming = self.stream_callback.is_some() && tools.is_none();

        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages,
            stream: use_streaming,
            tools,
            options: Some(OllamaOptions {
                num_ctx: Some(self.model_options.num_ctx),
                temperature: Some(self.model_options.temperature),
                top_p: self.model_options.top_p,
                top_k: self.model_options.top_k,
                num_predict: Some(self.model_options.num_predict),
            }),
        };

        if use_streaming {
            let future = self.send_streaming(body);
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
                    .map_err(|_| RuntimeError::new("ollama streaming thread panicked"))?
                })
            } else {
                let rt = tokio::runtime::Runtime::new()
                    .map_err(|e| RuntimeError::new(e.to_string()))?;
                rt.block_on(future)
            }
        } else {
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
}

impl OllamaBackend {
    async fn send_with_retry(
        &self,
        body: OllamaChatRequest,
    ) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let mut last_error = RuntimeError::new("no attempts made");

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                // Longer delay on empty response (cold start) — model is loading into GPU
                let delay = if last_error.to_string().contains("empty response") {
                    Duration::from_secs(3 + u64::from(attempt))
                } else {
                    Duration::from_millis(500 * u64::from(attempt))
                };
                tokio::time::sleep(delay).await;
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

    /// Streaming mode: read NDJSON chunks and call the callback for each token.
    /// Falls back to non-streaming on error.
    async fn send_streaming(
        &self,
        body: OllamaChatRequest,
    ) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::new(format!("ollama streaming request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(RuntimeError::new(format!("ollama returned {status}: {text}")));
        }

        let mut full_content = String::new();
        let mut prompt_eval_count = 0u32;
        let mut eval_count = 0u32;

        // Read the response body as text and parse line by line
        let body_text = response.text().await
            .map_err(|e| RuntimeError::new(format!("ollama stream read error: {e}")))?;

        for line in body_text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<OllamaStreamChunk>(line) {
                if !chunk.message.content.is_empty() {
                    // Call the streaming callback for real-time output
                    if let Some(cb) = &self.stream_callback {
                        cb(&chunk.message.content);
                    }
                    full_content.push_str(&chunk.message.content);
                }
                if chunk.done {
                    prompt_eval_count = chunk.prompt_eval_count.unwrap_or(0);
                    eval_count = chunk.eval_count.unwrap_or(0);
                }
            }
        }

        // Strip thinking blocks from accumulated content
        let clean = strip_thinking_blocks(&full_content);

        let mut events = Vec::new();
        if !clean.is_empty() {
            // Try to repair tool calls from text
            if let Some(repaired) = try_repair_tool_call_from_text(&clean) {
                events.push(repaired);
            } else {
                events.push(AssistantEvent::TextDelta(clean));
            }
        }

        events.push(AssistantEvent::Usage(TokenUsage {
            input_tokens: prompt_eval_count,
            output_tokens: eval_count,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }));
        events.push(AssistantEvent::MessageStop);

        if !events.iter().any(|e| matches!(e, AssistantEvent::TextDelta(_) | AssistantEvent::ToolUse { .. })) {
            return Err(RuntimeError::new("The model returned an empty response."));
        }

        Ok(events)
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
            // Emit text before tool calls if present (strip thinking blocks)
            if !message.content.is_empty() {
                let clean = strip_thinking_blocks(&message.content);
                if !clean.is_empty() {
                    events.push(AssistantEvent::TextDelta(clean));
                }
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
        // Strip Gemma 4 thinking blocks from output
        let clean_content = strip_thinking_blocks(&message.content);

        // Check if the model printed a tool call as text (common with local models)
        if let Some(repaired) = try_repair_tool_call_from_text(&clean_content) {
            events.push(repaired);
        } else if !clean_content.is_empty() {
            events.push(AssistantEvent::TextDelta(clean_content));
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
            "The model returned an empty response. This can happen when:\n\
             • The model is still loading (try again in a few seconds)\n\
             • The prompt is too long for the model's context window\n\
             • The model doesn't support tool calling\n\
             Try: tachy doctor to check model status",
        ));
    }

    Ok(events)
}

/// Strip Gemma 4 thinking/reasoning blocks from model output.
/// Gemma 4 uses `<|channel>thought\n...<channel|>` for internal reasoning.
/// Other models may use `<think>...</think>` (Qwen3, DeepSeek).
fn strip_thinking_blocks(text: &str) -> String {
    let mut result = text.to_string();

    // Gemma 4 format: <|channel>thought\n...<channel|>
    while let Some(start) = result.find("<|channel>thought") {
        if let Some(end) = result[start..].find("<channel|>") {
            let end_abs = start + end + "<channel|>".len();
            result = format!("{}{}", &result[..start], &result[end_abs..]);
        } else {
            // Unclosed thinking block — strip from start to end
            result = result[..start].to_string();
            break;
        }
    }

    // Common format: <think>...</think> (Qwen3, DeepSeek-R1)
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result[start..].find("</think>") {
            let end_abs = start + end + "</think>".len();
            result = format!("{}{}", &result[..start], &result[end_abs..]);
        } else {
            result = result[..start].to_string();
            break;
        }
    }

    result.trim().to_string()
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

    let known_tools = ["bash", "read_file", "write_file", "edit_file", "glob_search", "grep_search", "list_directory"];

    // Pattern 1: {"name": "bash", "parameters": {"command": "ls"}}
    if let (Some(name), Some(params)) = (
        obj.get("name").and_then(|v| v.as_str()),
        obj.get("parameters"),
    ) {
        if known_tools.contains(&name) {
            return Some(AssistantEvent::ToolUse {
                id: "repaired-0".to_string(),
                name: name.to_string(),
                input: serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string()),
            });
        }
    }

    // Pattern 2: {"name": "bash", "arguments": {"command": "ls"}} (OpenAI-style)
    if let (Some(name), Some(args)) = (
        obj.get("name").and_then(|v| v.as_str()),
        obj.get("arguments"),
    ) {
        if known_tools.contains(&name) {
            return Some(AssistantEvent::ToolUse {
                id: "repaired-0".to_string(),
                name: name.to_string(),
                input: serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
            });
        }
    }

    // Pattern 3: {"tool": "bash", "input": {"command": "ls"}} (alternate format)
    if let (Some(name), Some(input)) = (
        obj.get("tool").and_then(|v| v.as_str()),
        obj.get("input"),
    ) {
        if known_tools.contains(&name) {
            return Some(AssistantEvent::ToolUse {
                id: "repaired-0".to_string(),
                name: name.to_string(),
                input: serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()),
            });
        }
    }

    // Pattern 4: {"command": "ls"} — assume bash if it has a "command" field
    if obj.contains_key("command") && !obj.contains_key("name") && !obj.contains_key("tool") {
        return Some(AssistantEvent::ToolUse {
            id: "repaired-0".to_string(),
            name: "bash".to_string(),
            input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
        });
    }

    // Pattern 5: {"path": "..."} — assume read_file
    if obj.contains_key("path") && obj.len() <= 3 && !obj.contains_key("name") && !obj.contains_key("tool") {
        // If path ends with / or doesn't have an extension, it's probably list_directory
        if let Some(path) = obj.get("path").and_then(|v| v.as_str()) {
            if path.ends_with('/') || (!path.contains('.') && !path.contains("Cargo") && !path.contains("Makefile")) {
                return Some(AssistantEvent::ToolUse {
                    id: "repaired-0".to_string(),
                    name: "list_directory".to_string(),
                    input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
                });
            }
        }
        return Some(AssistantEvent::ToolUse {
            id: "repaired-0".to_string(),
            name: "read_file".to_string(),
            input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
        });
    }

    // Pattern 6: {"pattern": "..."} — assume grep_search or glob_search
    if let Some(pattern) = obj.get("pattern").and_then(|v| v.as_str()) {
        // If it looks like a glob pattern (contains * or ?), use glob_search
        if pattern.contains('*') || pattern.contains('?') {
            return Some(AssistantEvent::ToolUse {
                id: "repaired-0".to_string(),
                name: "glob_search".to_string(),
                input: serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string()),
            });
        }
        // Otherwise assume grep_search
        return Some(AssistantEvent::ToolUse {
            id: "repaired-0".to_string(),
            name: "grep_search".to_string(),
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
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
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

/// A single chunk from Ollama's streaming NDJSON response.
#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: OllamaStreamMessage,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    #[serde(default)]
    content: String,
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
    fn strips_gemma4_thinking_blocks() {
        let text = "<|channel>thought\nLet me think about this...\nI should read the file first.<channel|>Here is the answer.";
        assert_eq!(strip_thinking_blocks(text), "Here is the answer.");
    }

    #[test]
    fn strips_qwen_thinking_blocks() {
        let text = "<think>I need to analyze this carefully.</think>The result is 42.";
        assert_eq!(strip_thinking_blocks(text), "The result is 42.");
    }

    #[test]
    fn strips_multiple_thinking_blocks() {
        let text = "<think>first thought</think>middle<think>second thought</think>end";
        assert_eq!(strip_thinking_blocks(text), "middleend");
    }

    #[test]
    fn preserves_text_without_thinking_blocks() {
        let text = "Just a normal response with no thinking.";
        assert_eq!(strip_thinking_blocks(text), text);
    }

    #[test]
    fn does_not_repair_unknown_tool_json() {
        let text = r#"{"name": "unknown_tool", "parameters": {}}"#;
        assert!(try_repair_tool_call_from_text(text).is_none());
    }

    #[test]
    fn repairs_openai_style_arguments_format() {
        let text = r#"{"name": "bash", "arguments": {"command": "pwd"}}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "bash");
                assert!(input.contains("pwd"));
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn repairs_tool_input_format() {
        let text = r#"{"tool": "read_file", "input": {"path": "main.rs"}}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, input, .. } => {
                assert_eq!(name, "read_file");
                assert!(input.contains("main.rs"));
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn repairs_glob_pattern() {
        let text = r#"{"pattern": "**/*.rs"}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, .. } => {
                assert_eq!(name, "glob_search");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn repairs_grep_pattern() {
        let text = r#"{"pattern": "TODO"}"#;
        let event = try_repair_tool_call_from_text(text);
        assert!(event.is_some());
        match event.unwrap() {
            AssistantEvent::ToolUse { name, .. } => {
                assert_eq!(name, "grep_search");
            }
            _ => panic!("expected ToolUse"),
        }
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
