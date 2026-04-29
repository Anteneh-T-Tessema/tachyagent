//! `ConversationRuntime` — the agent turn-execution loop, plus `StaticToolExecutor`.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::types::{
    ApiClient, ApiRequest, AssistantEvent, ResponseFormat, RuntimeError, RuntimeEvent, ToolError,
    ToolExecutor, TurnSummary,
};
use crate::compact::{compact_session, estimate_session_tokens, CompactionConfig, CompactionResult};
use crate::permissions::{PermissionOutcome, PermissionPolicy, PermissionPrompter};
use crate::session::{ContentBlock, ConversationMessage, Session};
use crate::usage::{TokenUsage, UsageTracker};

pub struct ConversationRuntime<C, T> {
    session: Session,
    api_client: C,
    tool_executor: T,
    permission_policy: PermissionPolicy,
    system_prompt: Vec<String>,
    max_iterations: usize,
    required_write_file_path: Option<String>,
    usage_tracker: UsageTracker,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<RuntimeEvent>>,
    semantic_cache: Option<Arc<crate::semantic_cache::SemanticCache>>,
    embedder: Option<Arc<dyn crate::semantic_cache::Embedder>>,
    pub current_reward_score: Option<f32>,
}

impl<C, T> ConversationRuntime<C, T>
where
    C: ApiClient,
    T: ToolExecutor,
{
    #[must_use]
    pub fn new(
        session: Session,
        api_client: C,
        tool_executor: T,
        permission_policy: PermissionPolicy,
        system_prompt: Vec<String>,
    ) -> Self {
        let usage_tracker = UsageTracker::from_session(&session);
        Self {
            session,
            api_client,
            tool_executor,
            permission_policy,
            system_prompt,
            max_iterations: 16,
            required_write_file_path: None,
            usage_tracker,
            event_tx: None,
            semantic_cache: None,
            embedder: None,
            current_reward_score: None,
        }
    }

    #[must_use]
    pub fn with_embedder(mut self, embedder: Arc<dyn crate::semantic_cache::Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    #[must_use]
    pub fn with_semantic_cache(mut self, cache: Arc<crate::semantic_cache::SemanticCache>) -> Self {
        self.semantic_cache = Some(cache);
        self
    }

    #[must_use]
    pub fn with_event_tx(mut self, tx: tokio::sync::mpsc::UnboundedSender<RuntimeEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Replace the event sender for the next turn (used by streaming REPL).
    pub fn set_event_tx(&mut self, tx: tokio::sync::mpsc::UnboundedSender<RuntimeEvent>) {
        self.event_tx = Some(tx);
    }

    /// Restore a session loaded from disk (used for auto-resume).
    pub fn restore_session(&mut self, session: Session) {
        self.usage_tracker = UsageTracker::from_session(&session);
        self.session = session;
    }

    #[must_use]
    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    #[must_use]
    pub fn with_required_write_file_path(mut self, path: Option<String>) -> Self {
        self.required_write_file_path = path;
        self
    }

    /// Manually trigger session compaction with a custom summary.
    pub fn compact_with_summary(&mut self, summary: String) {
        let preserve_count = 4;
        let keep_from = self.session.messages.len().saturating_sub(preserve_count);
        let preserved = self.session.messages[keep_from..].to_vec();
        
        let continuation = crate::compact::get_compact_continuation_message(&summary, true, !preserved.is_empty());

        let mut compacted_messages = vec![ConversationMessage {
            role: crate::session::MessageRole::System,
            blocks: vec![ContentBlock::Text { text: continuation }],
            usage: None,
        }];
        compacted_messages.extend(preserved);
        
        self.session.messages = compacted_messages;
        
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(RuntimeEvent::SessionCompacted { 
                removed_count: keep_from,
                summary: summary.clone() 
            });
        }
    }

    pub fn run_turn(
        &mut self,
        user_input: impl Into<String>,
        mut prompter: Option<&mut dyn PermissionPrompter>,
    ) -> Result<TurnSummary, RuntimeError> {
        self.session
            .messages
            .push(ConversationMessage::user_text(user_input.into()));

        // Auto-compact if context is getting large (prevents hitting context window limits)
        self.auto_compact_if_needed();

        let mut assistant_messages = Vec::new();
        let mut tool_results = Vec::new();
        let mut iterations = 0;
        let mut consecutive_errors = 0;
        const MAX_CONSECUTIVE_ERRORS: u32 = 3;

        loop {
            iterations += 1;
            if iterations > self.max_iterations {
                // Return partial results instead of hard error
                if !assistant_messages.is_empty() {
                    break;
                }
                return Err(RuntimeError::new(
                    "conversation loop exceeded the maximum number of iterations",
                ));
            }

            let system_prompt_str = self.system_prompt.join("\n");
            let prompt_str = serde_json::to_string(&self.session.messages).unwrap_or_default();

            let mut events_cached = None;
            let mut is_cache_hit = false;
            let mut prompt_embedding = None;

            if let Some(cache) = &self.semantic_cache {
                // 1. Exact Match Lookup
                if let Some(cached) = cache.lookup(&prompt_str, &system_prompt_str) {
                    if let Some(tx) = &self.event_tx {
                        let _ = tx.send(RuntimeEvent::TextDelta(format!("[Exact Cache Hit] {}", cached.output)));
                    }
                    events_cached = Some(vec![
                        AssistantEvent::TextDelta(cached.output.clone()),
                        AssistantEvent::MessageStop,
                    ]);
                    is_cache_hit = true;
                } 
                // 2. Semantic Match Lookup (if exact fails and embedder is present)
                else if let Some(embedder) = &self.embedder {
                    if let Ok(emb) = embedder.embed(&prompt_str) {
                        prompt_embedding = Some(emb.clone());
                        if let Some(cached) = cache.lookup_semantic(&emb, 0.95) {
                            if let Some(trace) = &cached.expert_trace {
                                if let Some(tx) = &self.event_tx {
                                    let _ = tx.send(RuntimeEvent::TextDelta(format!("\n[Flash Guidance Active] Injecting expert trace from similar task...")));
                                }
                                // Inject expert trace as guidance in the system prompt
                                let guidance = format!("\n\n### EXPERT GUIDANCE\nSimilar task succeeded with this pattern:\n{}\nUse this as a guide for your execution.", trace);
                                self.system_prompt.push(guidance); self.current_reward_score = Some(cached.reward_score);
                            } else {
                                // Default semantic hit: return output
                                if let Some(tx) = &self.event_tx {
                                    let _ = tx.send(RuntimeEvent::TextDelta(format!("[Semantic Cache Hit] {}", cached.output)));
                                }
                                events_cached = Some(vec![
                                    AssistantEvent::TextDelta(cached.output.clone()),
                                    AssistantEvent::MessageStop,
                                ]);
                                is_cache_hit = true;
                            }
                        }
                    }
                }
            }

            let events = if let Some(e) = events_cached { e } else {
                // API call with error recovery
                match self.api_client.stream(ApiRequest {
                    system_prompt: self.system_prompt.clone(),
                    messages: self.session.messages.clone(),
                    format: ResponseFormat::default(),
                }) {
                    Ok(events) => {
                        consecutive_errors = 0;
                        // Emit TextDeltas if we have a listener
                        if let Some(tx) = &self.event_tx {
                            for event in &events {
                                if let AssistantEvent::TextDelta(delta) = event {
                                    let _ = tx.send(RuntimeEvent::TextDelta(delta.clone()));
                                }
                            }
                        }
                        events
                    }

                    Err(error) => {
                        consecutive_errors += 1;
                        if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                            // If we have partial results, return them
                            if !assistant_messages.is_empty() {
                                break;
                            }
                            return Err(error);
                        }
                        // Inject a recovery hint so the model can try again
                        self.session.messages.push(ConversationMessage::user_text(
                            format!("[System: Previous response failed ({error}). Please try again. If you need to use a tool, use the function calling mechanism — do not print JSON as text.]")
                        ));
                        continue;
                    }
                }
            };

            let (assistant_message, usage) = match build_assistant_message(events) {
                Ok(result) => {
                    if !is_cache_hit {
                        if let Some(cache) = &self.semantic_cache {
                            let text = assistant_message_to_text(&result.0);
                            cache.store(&prompt_str, &system_prompt_str, &text, "frontier", prompt_embedding, None, 1.0);
                        }
                    }
                    result
                },
                Err(error) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        if !assistant_messages.is_empty() {
                            break;
                        }
                        return Err(error);
                    }
                    // Inject recovery hint for malformed responses
                    self.session.messages.push(ConversationMessage::user_text(
                        "[System: Your previous response was malformed. Please respond with either plain text or a tool call. Do not mix formats.]".to_string()
                    ));
                    continue;
                }
            };

            if let Some(usage) = usage {
                self.usage_tracker.record(usage);
            }
            let pending_tool_uses = assistant_message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        if let Some(tx) = &self.event_tx {
                            let _ = tx.send(RuntimeEvent::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                            });
                        }
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();

            self.session.messages.push(assistant_message.clone());
            assistant_messages.push(assistant_message);

            if pending_tool_uses.is_empty() {
                if tool_results.is_empty() {
                    if let Some(path) = self.required_write_file_path.clone() {
                        let synthesized = self.synthesize_required_write_file(&assistant_messages, &path)?;
                        if let Some(result_message) = synthesized {
                            self.session.messages.push(result_message.clone());
                            if let Some(tx) = &self.event_tx {
                                if let ContentBlock::ToolResult { tool_name, output, is_error, .. } = &result_message.blocks[0] {
                                    let _ = tx.send(RuntimeEvent::ToolResult {
                                        tool_name: tool_name.clone(),
                                        output: output.clone(),
                                        is_error: *is_error,
                                    });
                                }
                            }
                            tool_results.push(result_message);
                        }
                    }
                }
                break;
            }

            for (tool_use_id, tool_name, input) in pending_tool_uses {
                let permission_outcome = if let Some(prompt) = prompter.as_mut() {
                    self.permission_policy
                        .authorize(&tool_name, &input, Some(*prompt), self.current_reward_score)
                } else {
                    self.permission_policy.authorize(&tool_name, &input, None, self.current_reward_score)
                };

                let result_message = match permission_outcome {
                    PermissionOutcome::Allow => {
                        match self.tool_executor.execute(&tool_name, &input) {
                            Ok(output) => {
                                // Truncate large tool outputs to prevent context overflow
                                let truncated = truncate_output(&output, 16_000);
                                ConversationMessage::tool_result(
                                    tool_use_id,
                                    tool_name,
                                    truncated,
                                    false,
                                )
                            }
                            Err(error) => {
                                let err_msg = error.to_string();
                                // Smart recovery for edit_file: if old_string not found,
                                // read the file and include its content so the model can
                                // see what the file actually contains and retry correctly.
                                let recovery = if tool_name == "edit_file" && err_msg.contains("not found") {
                                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&input) {
                                        if let Some(path) = parsed.get("path").and_then(|v| v.as_str()) {
                                            if let Ok(content) = std::fs::read_to_string(path) {
                                                let preview = if content.len() > 4000 {
                                                    format!("{}…\n[truncated, {} total chars]", &content[..4000], content.len())
                                                } else {
                                                    content
                                                };
                                                Some(format!(
                                                    "{err_msg}\n\nHere is the actual file content of {path}:\n```\n{preview}\n```\nPlease retry with the correct old_string that matches the file exactly."
                                                ))
                                            } else { None }
                                        } else { None }
                                    } else { None }
                                } else { None };

                                ConversationMessage::tool_result(
                                    tool_use_id,
                                    tool_name,
                                    recovery.unwrap_or(err_msg),
                                    true,
                                )
                            }
                        }
                    }
                    PermissionOutcome::Deny { reason } => {
                        ConversationMessage::tool_result(tool_use_id, tool_name, reason, true)
                    }
                };
                self.session.messages.push(result_message.clone());

                // Detect compaction trigger
                if let ContentBlock::ToolResult { output, .. } = &result_message.blocks[0] {
                    if output == "__TACTY_TRIGGER_COMPACTION__" {
                        let summary = crate::compact::digest_session(&self.session);
                        self.compact_with_summary(summary);
                    }
                }

                if let Some(tx) = &self.event_tx {
                    if let ContentBlock::ToolResult { tool_name, output, is_error, .. } = &result_message.blocks[0] {
                        let _ = tx.send(RuntimeEvent::ToolResult {
                            tool_name: tool_name.clone(),
                            output: output.clone(),
                            is_error: *is_error,
                        });
                    }
                }
                tool_results.push(result_message);
            }
        }

        let summary = TurnSummary {
            assistant_messages,
            tool_results,
            iterations,
            usage: self.usage_tracker.cumulative_usage(),
        };

        if let Some(tx) = &self.event_tx {
            let _ = tx.send(RuntimeEvent::Usage(summary.usage));
            let _ = tx.send(RuntimeEvent::Finished(summary.clone()));
        }

        Ok(summary)
    }

    #[must_use]
    pub fn compact(&self, config: CompactionConfig) -> CompactionResult {
        compact_session(&self.session, config)
    }

    /// Automatically compact the session if estimated tokens exceed a threshold.
    /// This prevents hitting context window limits mid-conversation.
    fn auto_compact_if_needed(&mut self) {
        let estimated = estimate_session_tokens(&self.session);
        // Compact when we're using more than ~6K tokens (conservative for local models)
        // Preserve the last 6 messages to maintain coherence
        if estimated > 6_000 && self.session.messages.len() > 8 {
            let config = CompactionConfig {
                preserve_recent_messages: 6,
                max_estimated_tokens: 6_000,
            };
            let result = compact_session(&self.session, config);
            if result.removed_message_count > 0 {
                self.session = result.compacted_session;
            }
        }
    }

    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        estimate_session_tokens(&self.session)
    }

    #[must_use]
    pub fn usage(&self) -> &UsageTracker {
        &self.usage_tracker
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    #[must_use]
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    #[must_use]
    pub fn into_session(self) -> Session {
        self.session
    }

    fn synthesize_required_write_file(
        &mut self,
        assistant_messages: &[ConversationMessage],
        output_path: &str,
    ) -> Result<Option<ConversationMessage>, RuntimeError> {
        let content = extract_assistant_text(assistant_messages);
        let content = normalize_required_write_content(&content);
        if content.trim().is_empty() {
            return Ok(None);
        }

        let input = serde_json::json!({
            "path": output_path,
            "content": content,
        })
        .to_string();

        match self.permission_policy.authorize("write_file", &input, None, self.current_reward_score) {
            PermissionOutcome::Allow => match self.tool_executor.execute("write_file", &input) {
                Ok(output) => Ok(Some(ConversationMessage::tool_result(
                    "required-artifact",
                    "write_file",
                    truncate_output(&output, 16_000),
                    false,
                ))),
                Err(error) => Err(RuntimeError::new(format!("required write_file failed: {error}"))),
            },
            PermissionOutcome::Deny { reason } => Err(RuntimeError::new(format!("required write_file denied: {reason}"))),
        }
    }
}

/// Build an assistant `ConversationMessage` from a stream of `AssistantEvent`s.
fn build_assistant_message(
    events: Vec<AssistantEvent>,
) -> Result<(ConversationMessage, Option<TokenUsage>), RuntimeError> {
    let mut text = String::new();
    let mut blocks = Vec::new();
    let mut finished = false;
    let mut usage = None;

    for event in events {
        match event {
            AssistantEvent::TextDelta(delta) => text.push_str(&delta),
            AssistantEvent::ToolUse { id, name, input } => {
                flush_text_block(&mut text, &mut blocks);
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
            AssistantEvent::Usage(value) => usage = Some(value),
            AssistantEvent::MessageStop => {
                finished = true;
            }
        }
    }

    flush_text_block(&mut text, &mut blocks);

    if !finished {
        return Err(RuntimeError::new(
            "assistant stream ended without a message stop event",
        ));
    }
    if blocks.is_empty() {
        return Err(RuntimeError::new("assistant stream produced no content"));
    }

    Ok((
        ConversationMessage::assistant_with_usage(blocks, usage),
        usage,
    ))
}

fn flush_text_block(text: &mut String, blocks: &mut Vec<ContentBlock>) {
    if !text.is_empty() {
        blocks.push(ContentBlock::Text {
            text: std::mem::take(text),
        });
    }
}

fn assistant_message_to_text(msg: &ConversationMessage) -> String {
    msg.blocks.iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_assistant_text(messages: &[ConversationMessage]) -> String {
    messages
        .iter()
        .flat_map(|msg| msg.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_required_write_content(text: &str) -> String {
    let marker = "\nCall write_file tool with the final markdown memo:";
    let text = text.split(marker).next().unwrap_or(text);
    text.trim().to_string()
}

/// Truncate tool output to prevent context window overflow.
fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.len() <= max_chars {
        return output.to_string();
    }
    // Keep the first portion and last portion for context
    let head = max_chars * 3 / 4;
    let tail = max_chars / 4;
    let tail_start = output.len().saturating_sub(tail);
    format!(
        "{}\n\n[… truncated {} chars …]\n\n{}",
        &output[..head],
        output.len() - head - tail,
        &output[tail_start..]
    )
}

// ── StaticToolExecutor ────────────────────────────────────────────────────────

type ToolHandler = Box<dyn FnMut(&str) -> Result<String, ToolError>>;

/// A `ToolExecutor` backed by a static map of named handler closures.
/// Useful for tests and simple CLI tools.
#[derive(Default)]
pub struct StaticToolExecutor {
    handlers: BTreeMap<String, ToolHandler>,
}

impl StaticToolExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn register(
        mut self,
        tool_name: impl Into<String>,
        handler: impl FnMut(&str) -> Result<String, ToolError> + 'static,
    ) -> Self {
        self.handlers.insert(tool_name.into(), Box::new(handler));
        self
    }
}

impl ToolExecutor for StaticToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        self.handlers
            .get_mut(tool_name)
            .ok_or_else(|| ToolError::new(format!("unknown tool: {tool_name}")))?(input)
    }
}
