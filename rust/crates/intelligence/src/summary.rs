//! Summary Agent for Context Compression.
//!
//! Uses Flash Distillation patterns to condense long conversation histories
//! into high-density state summaries.

use crate::finetune::FlashCompactor;
use serde::{Deserialize, Serialize};

/// A compressed snapshot of a conversation's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// The actual distilled summary text.
    pub content: String,
    /// The message index up to which this summary covers.
    pub last_message_idx: usize,
    /// The cryptographic hash of the session at the time of summary (for audit).
    pub session_hash: String,
}

pub struct SummaryManager;

impl SummaryManager {
    /// Distill a list of messages into a single "State Summary".
    ///
    /// This uses the `FlashCompactor` to extract logic and tool sequences
    /// while discarding chatty overhead.
    #[must_use]
    pub fn summarize(messages: &[serde_json::Value]) -> String {
        let mut summary = String::from("Current Session State:\n");

        // Wrap in a dummy session for FlashCompactor
        let dummy_session = serde_json::json!({
            "success": true,
            "messages": messages
        });

        if let Some(distilled) = FlashCompactor::distill_to_level(
            &dummy_session,
            crate::finetune::CompactionLevel::Logic,
        ) {
            summary.push_str(&distilled);
        } else {
            summary.push_str("No logic extracted yet.");
        }

        if summary.len() > 500 {
            summary.truncate(500);
            summary.push_str("...");
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_extracts_logic() {
        let messages = vec![
            json!({"role": "user", "content": "fix the bug"}),
            json!({"role": "assistant", "content": "I will search for the bug.\n```bash\ngrep -r \"bug\" .\n```"}),
        ];

        let summary = SummaryManager::summarize(&messages);
        assert!(summary.contains("Session State"));
        assert!(summary.contains("search for the bug"));
    }
}
