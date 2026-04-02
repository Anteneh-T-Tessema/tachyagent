use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, MessageRole,
    RuntimeError, TokenUsage,
};
use serde::{Deserialize, Serialize};
use tools::mvp_tool_specs;

pub struct OpenAiCompatBackend {
    client: reqwest::Client,
    base_url: String,
    model: String,
    enable_tools: bool,
}

impl OpenAiCompatBackend {
    pub fn new(
        model: String,
        base_url: String,
        api_key: Option<String>,
        enable_tools: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut headers = reqwest::header::HeaderMap::new();
        // Try config key, then OPENAI_API_KEY env, then GEMINI_API_KEY env
        let effective_key = api_key
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .or_else(|| std::env::var("GEMINI_API_KEY").ok());
        if let Some(key) = effective_key {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {key}")
                    .parse()
                    .map_err(|e: reqwest::header::InvalidHeaderValue| e.to_string())?,
            );
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self {
            client,
            base_url,
            model,
            enable_tools,
        })
    }
}

impl ApiClient for OpenAiCompatBackend {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let messages = convert_to_openai_messages(&request);
        let tools = if self.enable_tools {
            Some(build_openai_tools())
        } else {
            None
        };

        let body = OpenAiChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
            tools,
            tool_choice: if self.enable_tools {
                Some("auto".to_string())
            } else {
                None
            },
        };

        let future = self.send_request(body);

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
                .map_err(|_| RuntimeError::new("openai-compat request thread panicked"))?
            })
        } else {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| RuntimeError::new(e.to_string()))?;
            rt.block_on(future)
        }
    }
}

impl OpenAiCompatBackend {
    async fn send_request(&self, body: OpenAiChatRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::new(format!("openai-compat request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(RuntimeError::new(format!(
                "openai-compat returned {status}: {text}"
            )));
        }

        let chat: OpenAiChatResponse = response
            .json()
            .await
            .map_err(|e| RuntimeError::new(format!("openai-compat parse error: {e}")))?;

        parse_openai_response(chat)
    }
}

fn parse_openai_response(
    response: OpenAiChatResponse,
) -> Result<Vec<AssistantEvent>, RuntimeError> {
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| RuntimeError::new("openai-compat returned no choices"))?;

    let mut events = Vec::new();

    if let Some(content) = &choice.message.content {
        if !content.is_empty() {
            events.push(AssistantEvent::TextDelta(content.clone()));
        }
    }

    if let Some(tool_calls) = &choice.message.tool_calls {
        for call in tool_calls {
            events.push(AssistantEvent::ToolUse {
                id: call.id.clone(),
                name: call.function.name.clone(),
                input: call.function.arguments.clone(),
            });
        }
    }

    if let Some(usage) = response.usage {
        events.push(AssistantEvent::Usage(TokenUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        }));
    }

    events.push(AssistantEvent::MessageStop);

    if events.len() <= 1 {
        return Err(RuntimeError::new("openai-compat returned empty response"));
    }

    Ok(events)
}

fn convert_to_openai_messages(request: &ApiRequest) -> Vec<OpenAiMessage> {
    let mut messages = Vec::new();

    if !request.system_prompt.is_empty() {
        messages.push(OpenAiMessage {
            role: "system".to_string(),
            content: Some(request.system_prompt.join("\n\n")),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for msg in &request.messages {
        match msg.role {
            MessageRole::System => {
                let text = extract_text(&msg.blocks);
                if !text.is_empty() {
                    messages.push(OpenAiMessage {
                        role: "system".to_string(),
                        content: Some(text),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
            MessageRole::User => {
                let text = extract_text(&msg.blocks);
                if !text.is_empty() {
                    messages.push(OpenAiMessage {
                        role: "user".to_string(),
                        content: Some(text),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
            MessageRole::Assistant => {
                let text = extract_text(&msg.blocks);
                let tool_calls = extract_openai_tool_calls(&msg.blocks);
                messages.push(OpenAiMessage {
                    role: "assistant".to_string(),
                    content: if text.is_empty() { None } else { Some(text) },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                });
            }
            MessageRole::Tool => {
                for block in &msg.blocks {
                    if let ContentBlock::ToolResult {
                        tool_use_id,
                        output,
                        ..
                    } = block
                    {
                        messages.push(OpenAiMessage {
                            role: "tool".to_string(),
                            content: Some(output.clone()),
                            tool_calls: None,
                            tool_call_id: Some(tool_use_id.clone()),
                        });
                    }
                }
            }
        }
    }

    messages
}

fn extract_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_openai_tool_calls(blocks: &[ContentBlock]) -> Vec<OpenAiToolCall> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(OpenAiToolCall {
                id: id.clone(),
                r#type: "function".to_string(),
                function: OpenAiFunctionCall {
                    name: name.clone(),
                    arguments: input.clone(),
                },
            }),
            _ => None,
        })
        .collect()
}

fn build_openai_tools() -> Vec<OpenAiToolDef> {
    mvp_tool_specs()
        .into_iter()
        .map(|spec| OpenAiToolDef {
            r#type: "function".to_string(),
            function: OpenAiFunctionDef {
                name: spec.name.to_string(),
                description: Some(spec.description.to_string()),
                parameters: spec.input_schema,
            },
        })
        .collect()
}

// --- OpenAI-compatible API types ---

#[derive(Debug, Serialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiToolDef {
    r#type: String,
    function: OpenAiFunctionDef,
}

#[derive(Debug, Serialize)]
struct OpenAiFunctionDef {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}
