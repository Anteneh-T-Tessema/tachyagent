//! Compaction logic for Tachy sessions.
//!
//! Uses Flash Distillation to condense conversation history into a
//! high-density "Memory Digest" that preserves intent and state.

use crate::finetune::{CompactionLevel, FlashCompactor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactManager;

impl CompactManager {
    /// Summarize a list of messages into a "Session Digest".
    ///
    /// This removes redundant chat and focuses on:
    /// 1. The original goal.
    /// 2. Key discoveries (`read_file`, grep results).
    /// 3. Successful tool executions.
    /// 4. Current pending state.
    #[must_use]
    pub fn digest(messages: &[serde_json::Value]) -> String {
        let mut digest = String::from("# Session Digest (Compacted)\n\n");

        // Wrap in a dummy session for FlashCompactor
        let dummy_session = serde_json::json!({
            "success": true,
            "messages": messages
        });

        // Level 1: Extraction of pure logic/reasoning
        if let Some(logic) =
            FlashCompactor::distill_to_level(&dummy_session, CompactionLevel::Logic)
        {
            digest.push_str("## Reasoning Trace\n");
            digest.push_str(&logic);
            digest.push('\n');
        }

        // Level 2: Sequence of tools executed
        if let Some(sequence) =
            FlashCompactor::distill_to_level(&dummy_session, CompactionLevel::Sequence)
        {
            digest.push_str("## Execution Sequence\n");
            digest.push_str(&sequence);
            digest.push('\n');
        }

        digest.push_str("\n*Note: Detailed message history has been pruned to save context.*");
        digest
    }
}
