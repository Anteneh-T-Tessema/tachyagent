use std::time::Duration;

use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, MessageRole, RuntimeError, TokenUsage,
};
use serde::{Deserialize, Serialize};
use tools::mvp_tool_specs;

/// Maximum retries for transient Ollama failures.
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
    model_options: ModelOptions,
    token_tx: Option<tokio::sync::mpsc::Sender<BackendEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackendEvent {
    Text(String),
    Thinking(String),
}

struct ModelOptions {
    num_ctx: u32,
    temperature: f32,
    top_p: Option<f32>,
    top_k: Option<u32>,
    num_predict: u32,
}

impl ModelOptions {
    fn for_model(model: &str) -> Self {
        let lower = model.to_lowercase();
        let ram_gb = detect_available_ram_gb();

        if lower.contains("gemma4") {
            Self {
                num_ctx: if ram_gb >= 48 {
                    32_768
                } else if ram_gb >= 24 {
                    16_384
                } else {
                    8192
                },
                temperature: 1.0,
                top_p: Some(0.95),
                top_k: Some(64),
                num_predict: if ram_gb >= 32 { 8192 } else { 4096 },
            }
        } else if lower.contains("qwen3") {
            Self {
                num_ctx: if ram_gb >= 24 { 16_384 } else { 8192 },
                temperature: 0.7,
                top_p: Some(0.8),
                top_k: None,
                num_predict: 4096,
            }
        } else {
            Self {
                num_ctx: if ram_gb >= 16 { 8192 } else { 4096 },
                temperature: 0.1,
                top_p: None,
                top_k: None,
                num_predict: 4096,
            }
        }
    }
}

fn detect_available_ram_gb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sysctl")
            .arg("-n")
            .arg("hw.memsize")
            .output()
        {
            if let Ok(bytes) = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<u64>()
            {
                return bytes / 1_073_741_824;
            }
        }
    }
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
            token_tx: None,
        })
    }

    pub fn set_token_tx(&mut self, tx: tokio::sync::mpsc::Sender<BackendEvent>) {
        self.token_tx = Some(tx);
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Return the correct FIM (Fill-in-the-Middle) tokens for this model.
    #[must_use]
    pub fn get_fim_tokens(&self) -> (&str, &str, &str) {
        let m = self.model.to_lowercase();
        if m.contains("codellama") || m.contains("llama-2") {
            ("<PRE>", "<SUF>", "<MID>")
        } else if m.contains("starcoder") || m.contains("stablecode") {
            ("<fim_prefix>", "<fim_suffix>", "<fim_middle>")
        } else {
            // Default to Gemma / DeepSeek / Llama 3 format
            ("<|fim_prefix|>", "<|fim_suffix|>", "<|fim_middle|>")
        }
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
        let use_streaming = self.token_tx.is_some() && tools.is_none();

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
            format: None,
        };

        if use_streaming {
            let (events, _metrics) = self.stream_internal(body)?;
            Ok(events)
        } else {
            let future = self.send_with_retry(body);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                // If we are already in a tokio runtime, we can't build a new one.
                // We use block_in_place if we are on a multi-threaded runtime, or just block if single-threaded.
                tokio::task::block_in_place(|| handle.block_on(future))
            } else {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| RuntimeError::new(e.to_string()))?;
                rt.block_on(future)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InferenceMetrics {
    pub ttft_ms: u32,
    pub tokens_per_sec: f32,
    pub total_tokens: u64,
}

impl OllamaBackend {
    pub fn generate(
        &mut self,
        prefix: &str,
        suffix: &str,
        max_tokens: u32,
    ) -> Result<(Vec<AssistantEvent>, InferenceMetrics), RuntimeError> {
        let is_gemma4 = self.model.to_lowercase().contains("gemma4");
        let prompt = if is_gemma4 {
            format!("<|fim_prefix|>{prefix}<|fim_suffix|>{suffix}<|fim_middle|>")
        } else {
            format!("{prefix}{suffix}")
        };

        let body = OllamaGenerateRequest {
            model: self.model.clone(),
            prompt,
            stream: self.token_tx.is_some(),
            raw: is_gemma4,
            options: Some(OllamaOptions {
                num_ctx: Some(self.model_options.num_ctx),
                temperature: Some(0.1),
                top_p: Some(0.9),
                top_k: Some(40),
                num_predict: Some(max_tokens),
            }),
        };

        let future = self.send_streaming_generate(body);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            rt.block_on(future)
        }
    }

    fn stream_internal(
        &self,
        body: OllamaChatRequest,
    ) -> Result<(Vec<AssistantEvent>, InferenceMetrics), RuntimeError> {
        let future = self.send_streaming(body);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            rt.block_on(future)
        }
    }

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
                    if e.to_string().contains("404") || e.to_string().contains("400") {
                        return Err(e);
                    }
                    last_error = e;
                }
            }
        }
        Err(RuntimeError::new(format!(
            "ollama failed after attempts: {last_error}"
        )))
    }

    pub async fn send_streaming(
        &self,
        body: OllamaChatRequest,
    ) -> Result<(Vec<AssistantEvent>, InferenceMetrics), RuntimeError> {
        let mut response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::new(format!("ollama request failed: {e}")))?;
        if !response.status().is_success() {
            return Err(RuntimeError::new(format!(
                "ollama returned {}",
                response.status()
            )));
        }

        let start = std::time::Instant::now();
        let mut ttft = None;
        let mut content = String::new();
        let mut p_tokens = 0;
        let mut e_tokens = 0;

        let mut buffer = Vec::new();

        while let Ok(Some(chunk)) = response.chunk().await {
            buffer.extend_from_slice(&chunk);

            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line = buffer.drain(..=pos).collect::<Vec<u8>>();
                if let Ok(chunk_data) = serde_json::from_slice::<OllamaStreamChunk>(&line) {
                    let is_thinking = chunk_data.message.thinking.is_some();
                    let text = chunk_data
                        .message
                        .thinking
                        .unwrap_or_else(|| chunk_data.message.content.clone());
                    if !text.is_empty() {
                        if ttft.is_none() {
                            ttft = Some(start.elapsed());
                        }
                        if let Some(tx) = &self.token_tx {
                            let event = if is_thinking {
                                BackendEvent::Thinking(text.clone())
                            } else {
                                BackendEvent::Text(text.clone())
                            };
                            let _ = tx.send(event).await;
                        }
                        content.push_str(&text);
                    }
                    if chunk_data.done {
                        p_tokens = chunk_data.prompt_eval_count.unwrap_or(0);
                        e_tokens = chunk_data.eval_count.unwrap_or(0);
                    }
                } else if !line.is_empty() {
                    // Log or handle malformed JSON line if needed
                }
            }
        }

        let ttft_ms = ttft.unwrap_or(start.elapsed()).as_millis() as u32;
        let elapsed = start.elapsed().as_secs_f32();
        let tps = if elapsed > 0.0 {
            e_tokens as f32 / elapsed
        } else {
            0.0
        };
        let clean = strip_thinking_blocks(&content);

        let mut events = Vec::new();
        if !clean.is_empty() {
            if let Some(repaired) = try_repair_tool_call_from_text(&clean) {
                events.push(repaired);
            } else {
                events.push(AssistantEvent::TextDelta(clean));
            }
        }
        events.push(AssistantEvent::Usage(TokenUsage {
            input_tokens: p_tokens,
            output_tokens: e_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }));
        events.push(AssistantEvent::MessageStop);

        Ok((
            events,
            InferenceMetrics {
                ttft_ms,
                tokens_per_sec: tps,
                total_tokens: u64::from(p_tokens + e_tokens),
            },
        ))
    }

    pub async fn send_streaming_generate(
        &self,
        body: OllamaGenerateRequest,
    ) -> Result<(Vec<AssistantEvent>, InferenceMetrics), RuntimeError> {
        let mut response = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::new(e.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::new(format!(
                "ollama prompt failed: {}",
                response.status()
            )));
        }

        let start = std::time::Instant::now();
        let mut ttft = None;
        let mut content = String::new();
        let mut p_tokens = 0;
        let mut e_tokens = 0;

        let mut buffer = Vec::new();

        while let Ok(Some(chunk)) = response.chunk().await {
            buffer.extend_from_slice(&chunk);

            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line = buffer.drain(..=pos).collect::<Vec<u8>>();
                #[derive(Deserialize)]
                struct GenChunk {
                    response: String,
                    done: bool,
                    prompt_eval_count: Option<u32>,
                    eval_count: Option<u32>,
                }
                if let Ok(chunk_data) = serde_json::from_slice::<GenChunk>(&line) {
                    if !chunk_data.response.is_empty() {
                        if ttft.is_none() {
                            ttft = Some(start.elapsed());
                        }
                        if let Some(tx) = &self.token_tx {
                            let _ = tx
                                .send(BackendEvent::Text(chunk_data.response.clone()))
                                .await;
                        }
                        content.push_str(&chunk_data.response);
                    }
                    if chunk_data.done {
                        p_tokens = chunk_data.prompt_eval_count.unwrap_or(0);
                        e_tokens = chunk_data.eval_count.unwrap_or(0);
                    }
                }
            }
        }

        let ttft_ms = ttft.unwrap_or(start.elapsed()).as_millis() as u32;
        let elapsed = start.elapsed().as_secs_f32();
        let tps = if elapsed > 0.0 {
            e_tokens as f32 / elapsed
        } else {
            0.0
        };
        let mut events = Vec::new();
        if !content.is_empty() {
            events.push(AssistantEvent::TextDelta(content));
        }
        events.push(AssistantEvent::Usage(TokenUsage {
            input_tokens: p_tokens,
            output_tokens: e_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }));
        events.push(AssistantEvent::MessageStop);

        Ok((
            events,
            InferenceMetrics {
                ttft_ms,
                tokens_per_sec: tps,
                total_tokens: u64::from(p_tokens + e_tokens),
            },
        ))
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
            .map_err(|e| RuntimeError::new(e.to_string()))?;
        if !response.status().is_success() {
            return Err(RuntimeError::new(format!(
                "ollama error: {}",
                response.status()
            )));
        }
        let chat_res: OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| RuntimeError::new(e.to_string()))?;
        parse_ollama_response(chat_res)
    }
}

fn parse_ollama_response(res: OllamaChatResponse) -> Result<Vec<AssistantEvent>, RuntimeError> {
    let mut events = Vec::new();
    let msg = res.message;

    if let Some(calls) = &msg.tool_calls {
        if !calls.is_empty() {
            if !msg.content.is_empty() {
                let clean = strip_thinking_blocks(&msg.content);
                if !clean.is_empty() {
                    events.push(AssistantEvent::TextDelta(clean));
                }
            }
            for (i, call) in calls.iter().enumerate() {
                events.push(AssistantEvent::ToolUse {
                    id: call.id.clone().unwrap_or_else(|| format!("ollama-{i}")),
                    name: call.function.name.clone(),
                    input: serde_json::to_string(&call.function.arguments).unwrap_or_default(),
                });
            }
        }
    }

    if !events
        .iter()
        .any(|e| matches!(e, AssistantEvent::ToolUse { .. }))
        && !msg.content.is_empty()
    {
        let clean = strip_thinking_blocks(&msg.content);
        if let Some(repaired) = try_repair_tool_call_from_text(&clean) {
            events.push(repaired);
        } else if !clean.is_empty() {
            events.push(AssistantEvent::TextDelta(clean));
        }
    }

    events.push(AssistantEvent::Usage(TokenUsage {
        input_tokens: res.prompt_eval_count.unwrap_or(0),
        output_tokens: res.eval_count.unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    }));
    events.push(AssistantEvent::MessageStop);

    if !events.iter().any(|e| {
        matches!(
            e,
            AssistantEvent::TextDelta(_) | AssistantEvent::ToolUse { .. }
        )
    }) {
        return Err(RuntimeError::new("empty response"));
    }
    Ok(events)
}

fn strip_thinking_blocks(text: &str) -> String {
    let mut res = text.to_string();
    while let Some(s) = res.find("<|channel>thought") {
        if let Some(e) = res[s..].find("<channel|>") {
            res = format!("{}{}", &res[..s], &res[s + e + 10..]);
        } else {
            res = res[..s].to_string();
            break;
        }
    }
    while let Some(s) = res.find("<think>") {
        if let Some(e) = res[s..].find("</think>") {
            res = format!("{}{}", &res[..s], &res[s + e + 8..]);
        } else {
            res = res[..s].to_string();
            break;
        }
    }
    res.trim().to_string()
}

fn try_repair_tool_call_from_text(text: &str) -> Option<AssistantEvent> {
    let trimmed = text.trim();
    let s = trimmed.find('{')?;
    let e = trimmed.rfind('}')?;
    let parsed: serde_json::Value = serde_json::from_str(&trimmed[s..=e]).ok()?;
    let obj = parsed.as_object()?;

    if let (Some(n), Some(p)) = (
        obj.get("name").and_then(|v| v.as_str()),
        obj.get("parameters").or(obj.get("arguments")),
    ) {
        return Some(AssistantEvent::ToolUse {
            id: "repaired".to_string(),
            name: n.to_string(),
            input: p.to_string(),
        });
    }
    None
}

fn convert_to_ollama_messages(req: &ApiRequest) -> Vec<OllamaMessage> {
    let mut msgs = Vec::new();
    if !req.system_prompt.is_empty() {
        msgs.push(OllamaMessage {
            role: "system".to_string(),
            content: req.system_prompt.join("\n"),
            tool_calls: None,
        });
    }
    for m in &req.messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        let content = extract_text_content(&m.blocks);
        msgs.push(OllamaMessage {
            role: role.to_string(),
            content,
            tool_calls: None,
        });
    }
    msgs
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

fn build_ollama_tools() -> Vec<OllamaTool> {
    mvp_tool_specs()
        .into_iter()
        .map(|s| OllamaTool {
            r#type: "function".to_string(),
            function: OllamaToolFunction {
                name: s.name.to_string(),
                description: s.description.to_string(),
                parameters: s.input_schema,
            },
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OllamaTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    pub stream: bool,
    pub raw: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolCall {
    pub id: Option<String>,
    pub function: OllamaFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaFunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaTool {
    pub r#type: String,
    pub function: OllamaToolFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
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
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: OllamaStreamMessage,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    content: String,
    #[serde(default)]
    thinking: Option<String>,
}
