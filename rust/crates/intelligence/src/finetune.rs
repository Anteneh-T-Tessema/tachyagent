//! Fine-tuning dataset extraction and `LoRA` workflow helpers.
//!
//! Converts Tachy session history into Alpaca-format JSONL suitable for
//! `LoRA` fine-tuning with Unsloth, Axolotl, or llama.cpp.
//!
//! Also generates:
//! - An Ollama `Modelfile` for packaging a custom-adapted model.
//! - A shell training script (informational; no actual training is run here).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Dataset types
// ---------------------------------------------------------------------------

/// A single Alpaca-format fine-tuning example.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneEntry {
    /// The instruction / user prompt.
    pub instruction: String,
    /// Optional supplementary context (empty for conversational pairs).
    pub input: String,
    /// The expected model response.
    pub output: String,
}

/// A collected dataset ready for `LoRA` training.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FinetuneDataset {
    pub entries: Vec<FinetuneEntry>,
    pub source_sessions: usize,
    pub total_pairs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainingStatus { Queued, Running, Completed, Failed }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingJob {
    pub id: String,
    pub status: TrainingStatus,
    pub adapter_name: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub error: Option<String>,
}

impl FinetuneDataset {
    /// Extract training pairs from all `.json` session files in `sessions_dir`, 
    /// isolating by `team_id` if provided.
    ///
    /// If `role_filter` is provided, only sessions using that agent template name are included.
    #[must_use] pub fn from_sessions_isolated(sessions_dir: &Path, gold_standard_only: bool, team_id: Option<&str>, role_filter: Option<&str>) -> Self {
        let mut dataset = FinetuneDataset::default();
        let Ok(entries) = std::fs::read_dir(sessions_dir) else {
            return dataset;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            
            // Check Team ID isolation if provided
            if let Some(tid) = team_id {
                let session_team = json["team_id"].as_str();
                if session_team != Some(tid) {
                    continue;
                }
            }

            // Check Role Filter if provided
            if let Some(role) = role_filter {
                let session_role = json["agent_name"].as_str();
                if session_role != Some(role) {
                    continue;
                }
            }
            
            // Check Gold Standard criteria if requested
            if gold_standard_only {
                let success = json["success"].as_bool().unwrap_or(false);
                let overriden = json["human_override"].as_bool().unwrap_or(false);
                if !success || overriden {
                    continue;
                }
            }

            let Some(messages) = json["messages"].as_array() else {
                continue;
            };

            dataset.source_sessions += 1;
            let mut last_user: Option<String> = None;

            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("");
                let text = extract_text_from_message(msg);
                if text.is_empty() {
                    continue;
                }
                match role {
                    "User" | "user" => {
                        last_user = Some(text);
                    }
                    "Assistant" | "assistant" => {
                        if let Some(user_text) = last_user.take() {
                            dataset.entries.push(FinetuneEntry {
                                instruction: user_text,
                                input: String::new(),
                                output: text,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        dataset.total_pairs = dataset.entries.len();
        dataset
    }

    /// Serialize the dataset to newline-delimited JSON (JSONL).
    #[must_use] pub fn to_jsonl(&self) -> String {
        self.entries
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Save the dataset as JSONL to `path`.
    pub fn save_jsonl(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_jsonl())
    }

    /// Check if enough "Gold Standard" sessions exist to justify a fine-tuning run.
    #[must_use] pub fn should_trigger(sessions_dir: &Path, threshold: usize) -> bool {
        let Ok(entries) = std::fs::read_dir(sessions_dir) else {
            return false;
        };

        let mut gold_count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let success = json["success"].as_bool().unwrap_or(false);
                let overriden = json["human_override"].as_bool().unwrap_or(false);
                if success && !overriden {
                    gold_count += 1;
                }
            }
            if gold_count >= threshold {
                return true;
            }
        }
        false
    }

    /// Prepare a complete training bundle (JSONL, Modelfile, train.sh) in the target directory.
    pub fn prepare_training_bundle(sessions_dir: &Path, output_dir: &Path, base_model: &str) -> std::io::Result<String> {
        let dataset = Self::from_sessions_isolated(sessions_dir, true, None, None);
        if dataset.entries.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No Gold Standard sessions found"));
        }

        std::fs::create_dir_all(output_dir)?;

        let jsonl_path = output_dir.join("dataset.jsonl");
        dataset.save_jsonl(&jsonl_path)?;

        let mf_content = generate_modelfile(base_model, "./adapter.gguf", "You are Tachy, a fast local AI coding agent optimized for this codebase.");
        std::fs::write(output_dir.join("Modelfile"), mf_content)?;

        let script_content = generate_training_script(base_model, "dataset.jsonl", ".");
        let script_path = output_dir.join("train.sh");
        std::fs::write(&script_path, script_content)?;

        Ok(output_dir.display().to_string())
    }
}

// ---------------------------------------------------------------------------
// Modelfile generation
// ---------------------------------------------------------------------------

/// Generate an Ollama `Modelfile` that packages a LoRA-adapted model.
///
/// # Arguments
/// * `base_model`    - e.g. `"mistral:7b"` or `"gemma:7b"`
/// * `adapter_path`  - path to the `.gguf` `LoRA` adapter produced by training
/// * `system_prompt` - custom system prompt baked into the model
#[must_use] pub fn generate_modelfile(base_model: &str, adapter_path: &str, system_prompt: &str) -> String {
    format!(
        r#"FROM {base_model}

# LoRA adapter trained on Tachy session history
ADAPTER {adapter_path}

SYSTEM """{system_prompt}
"""

PARAMETER temperature 0.3
PARAMETER num_ctx 8192
PARAMETER stop "<|end|>"
PARAMETER stop "<|user|>"
PARAMETER stop "<|assistant|>"
"#
    )
}

// ---------------------------------------------------------------------------
// Training script generation
// ---------------------------------------------------------------------------

/// Generate a shell script for running `LoRA` fine-tuning with Unsloth.
///
/// This is informational output only — no training is performed by Tachy.
/// The script is written to disk so the user can inspect, modify, and run it.
#[must_use] pub fn generate_training_script(model_id: &str, dataset_path: &str, output_dir: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
# Tachy-generated LoRA fine-tuning script
# Prerequisites: pip install unsloth torch trl datasets transformers
set -euo pipefail

MODEL_ID="{model_id}"
DATASET="{dataset_path}"
OUTPUT="{output_dir}"

echo "⚡ Tachy LoRA fine-tuning"
echo "  Base model : $MODEL_ID"
echo "  Dataset    : $DATASET"
echo "  Output dir : $OUTPUT"
echo ""

mkdir -p "$OUTPUT"

python3 - <<'EOF'
from unsloth import FastLanguageModel
import torch

model, tokenizer = FastLanguageModel.from_pretrained(
    model_name="{model_id}",
    max_seq_length=2048,
    load_in_4bit=True,
)

model = FastLanguageModel.get_peft_model(
    model,
    r=16,
    target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
    lora_alpha=16,
    lora_dropout=0.0,
    bias="none",
    use_gradient_checkpointing=True,
)

from trl import SFTTrainer
from datasets import load_dataset
from transformers import TrainingArguments

dataset = load_dataset("json", data_files={{"train": "{dataset_path}"}}, split="train")

trainer = SFTTrainer(
    model=model,
    tokenizer=tokenizer,
    train_dataset=dataset,
    dataset_text_field="instruction",
    max_seq_length=2048,
    args=TrainingArguments(
        per_device_train_batch_size=2,
        gradient_accumulation_steps=4,
        warmup_steps=5,
        max_steps=200,
        learning_rate=2e-4,
        fp16=not torch.cuda.is_bf16_supported(),
        bf16=torch.cuda.is_bf16_supported(),
        output_dir="{output_dir}",
        save_steps=50,
        logging_steps=10,
    ),
)
trainer.train()
model.save_pretrained_gguf("{output_dir}", tokenizer)
print("✓ LoRA adapter saved to {output_dir}")
EOF

echo ""
echo "Done! Next steps:"
echo "  1. Edit the generated Modelfile (path: {output_dir}/Modelfile)"
echo "  2. ollama create my-tachy-model -f {output_dir}/Modelfile"
echo "  3. tachy --model my-tachy-model"
"#
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract plain text from a session message JSON node.
///
/// Handles both the blocks-based format used by the Tachy runtime and the
/// flat `{ "content": "..." }` format.
fn extract_text_from_message(msg: &serde_json::Value) -> String {
    // Blocks-based format: {"blocks": [{"Text": {"text": "..."}}, ...]}
    if let Some(blocks) = msg["blocks"].as_array() {
        let parts: Vec<&str> = blocks
            .iter()
            .filter_map(|b| {
                b["Text"]["text"]
                    .as_str()
                    .or_else(|| b["text"].as_str())
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    // Flat content format
    if let Some(s) = msg["content"].as_str() {
        return s.to_string();
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(suffix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tachy-finetune-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_sessions_dir_returns_empty_dataset() {
        let dir = tmp_dir("empty");
        let ds = FinetuneDataset::from_sessions_isolated(&dir, false, None, None);
        assert_eq!(ds.entries.len(), 0);
        assert_eq!(ds.source_sessions, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pairs_user_assistant_turns() {
        let dir = tmp_dir("pairs");
        let session = serde_json::json!({
            "version": 1,
            "messages": [
                {"role": "user", "content": "What is Rust?"},
                {"role": "assistant", "content": "Rust is a systems language."},
                {"role": "user", "content": "Why is it fast?"},
                {"role": "assistant", "content": "Zero-cost abstractions."},
            ]
        });
        std::fs::write(
            dir.join("session1.json"),
            serde_json::to_string(&session).unwrap(),
        )
        .unwrap();
        let ds = FinetuneDataset::from_sessions_isolated(&dir, false, None, None);
        assert_eq!(ds.total_pairs, 2);
        assert_eq!(ds.entries[0].instruction, "What is Rust?");
        assert_eq!(ds.entries[0].output, "Rust is a systems language.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_non_json_files() {
        let dir = tmp_dir("skip-non-json");
        std::fs::write(dir.join("notes.txt"), "not a session").unwrap();
        let ds = FinetuneDataset::from_sessions_isolated(&dir, false, None, None);
        assert_eq!(ds.source_sessions, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn to_jsonl_produces_one_line_per_entry() {
        let ds = FinetuneDataset {
            entries: vec![
                FinetuneEntry {
                    instruction: "Q1".to_string(),
                    input: String::new(),
                    output: "A1".to_string(),
                },
                FinetuneEntry {
                    instruction: "Q2".to_string(),
                    input: String::new(),
                    output: "A2".to_string(),
                },
            ],
            source_sessions: 1,
            total_pairs: 2,
        };
        let jsonl = ds.to_jsonl();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn save_and_reload_jsonl() {
        let dir = tmp_dir("save-reload");
        let ds = FinetuneDataset {
            entries: vec![FinetuneEntry {
                instruction: "hello".to_string(),
                input: String::new(),
                output: "world".to_string(),
            }],
            source_sessions: 1,
            total_pairs: 1,
        };
        let out = dir.join("data.jsonl");
        ds.save_jsonl(&out).unwrap();
        let content = std::fs::read_to_string(&out).unwrap();
        let parsed: FinetuneEntry = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed.instruction, "hello");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn generate_modelfile_includes_key_fields() {
        let mf = generate_modelfile("mistral:7b", "./adapter.gguf", "You are a Rust expert.");
        assert!(mf.contains("FROM mistral:7b"));
        assert!(mf.contains("ADAPTER ./adapter.gguf"));
        assert!(mf.contains("You are a Rust expert."));
        assert!(mf.contains("PARAMETER temperature 0.3"));
    }

    #[test]
    fn generate_training_script_contains_model_id() {
        let script = generate_training_script("llama3:8b", "data.jsonl", "/tmp/out");
        assert!(script.contains("llama3:8b"));
        assert!(script.contains("data.jsonl"));
        assert!(script.contains("/tmp/out"));
    }

    #[test]
    fn gold_standard_filtering_works() {
        let dir = tmp_dir("gold");
        let s1 = serde_json::json!({
            "success": true,
            "messages": [{"role": "user", "content": "Q1"}, {"role": "assistant", "content": "A1"}]
        });
        let s2 = serde_json::json!({
            "success": false,
            "messages": [{"role": "user", "content": "Q2"}, {"role": "assistant", "content": "A2"}]
        });
        std::fs::write(dir.join("s1.json"), serde_json::to_string(&s1).unwrap()).unwrap();
        std::fs::write(dir.join("s2.json"), serde_json::to_string(&s2).unwrap()).unwrap();

        let ds_all = FinetuneDataset::from_sessions_isolated(&dir, false, None, None);
        assert_eq!(ds_all.total_pairs, 2);

        let ds_gold = FinetuneDataset::from_sessions_isolated(&dir, true, None, None);
        assert_eq!(ds_gold.total_pairs, 1);
        assert_eq!(ds_gold.entries[0].instruction, "Q1");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn team_isolation_filtering_works() {
        let dir = tmp_dir("iso");
        let s1 = serde_json::json!({
            "team_id": "teamA",
            "messages": [{"role": "user", "content": "QA"}, {"role": "assistant", "content": "AA"}]
        });
        let s2 = serde_json::json!({
            "team_id": "teamB",
            "messages": [{"role": "user", "content": "QB"}, {"role": "assistant", "content": "AB"}]
        });
        std::fs::write(dir.join("s1.json"), serde_json::to_string(&s1).unwrap()).unwrap();
        std::fs::write(dir.join("s2.json"), serde_json::to_string(&s2).unwrap()).unwrap();

        let ds_a = FinetuneDataset::from_sessions_isolated(&dir, false, Some("teamA"), None);
        assert_eq!(ds_a.total_pairs, 1);
        assert_eq!(ds_a.entries[0].instruction, "QA");

        let ds_b = FinetuneDataset::from_sessions_isolated(&dir, false, Some("teamB"), None);
        assert_eq!(ds_b.total_pairs, 1);
        assert_eq!(ds_b.entries[0].instruction, "QB");

        let ds_none = FinetuneDataset::from_sessions_isolated(&dir, false, Some("teamC"), None);
        assert_eq!(ds_none.total_pairs, 0);

        std::fs::remove_dir_all(dir).ok();
    }
}

// ---------------------------------------------------------------------------
// Quality Scorer — assigns a 0.0..1.0 quality score to a completed session.
// ---------------------------------------------------------------------------

/// Scores a completed session to determine if it qualifies as Gold Standard.
///
/// Criteria (each contributing independently):
/// - `success: true` — hard requirement; fails immediately if false
/// - `human_override: false` — hard requirement; overridden sessions are excluded
/// - Turn count in 2..=30 — optimal range signals purposeful, focused sessions
/// - Average response length >= 20 chars — guards against empty-output sessions
pub struct QualityScorer;

impl QualityScorer {
    /// Score a session JSON blob. Returns 0.0 on hard disqualification,
    /// otherwise 0.5..=1.0 depending on soft signals.
    #[must_use]
    pub fn score(session: &serde_json::Value) -> f32 {
        // Hard gates
        if !session["success"].as_bool().unwrap_or(false) { return 0.0; }
        if session["human_override"].as_bool().unwrap_or(false) { return 0.0; }

        let messages = match session["messages"].as_array() {
            Some(m) if !m.is_empty() => m,
            _ => return 0.0,
        };

        // Soft signals
        let turn_count = messages.len();
        let turn_score: f32 = if turn_count >= 2 && turn_count <= 30 {
            1.0 - ((turn_count as f32 - 10.0).abs() / 20.0).min(1.0) * 0.3
        } else {
            0.4
        };

        // Reward non-empty assistant responses
        let avg_len: f32 = {
            let assistant_texts: Vec<usize> = messages.iter()
                .filter(|m| matches!(m["role"].as_str(), Some("assistant" | "Assistant")))
                .filter_map(|m| m["content"].as_str().map(|s| s.len()))
                .collect();
            if assistant_texts.is_empty() {
                return 0.0;
            }
            assistant_texts.iter().sum::<usize>() as f32 / assistant_texts.len() as f32
        };
        let length_score: f32 = (avg_len / 200.0).min(1.0);

        0.4 + 0.35 * turn_score + 0.25 * length_score
    }

    /// Returns true when the session score meets the Gold Standard threshold (≥ 0.75).
    #[must_use]
    pub fn is_gold_standard(session: &serde_json::Value) -> bool {
        Self::score(session) >= 0.75
    }
}

// ---------------------------------------------------------------------------
// Gold Standard Store — durable append-only JSONL of approved training pairs.
// ---------------------------------------------------------------------------

/// Durable, append-only store for Gold Standard Alpaca-format training pairs.
///
/// Backed by `.tachy/gold_standard/dataset.jsonl`.  Writes are atomic per
/// append: each call opens the file in append mode, writes complete JSON lines,
/// then closes, so the file is always in a readable state.
pub struct GoldStandardStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldStandardStats {
    pub total_entries: usize,
    pub dataset_path: String,
    pub size_bytes: u64,
}

impl GoldStandardStore {
    /// Create a store backed by `path` (parent directories are created if needed).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        Self { path }
    }

    /// Score `session` and, if it qualifies as Gold Standard, extract and
    /// append its (instruction, output) pairs to the JSONL file.
    /// Returns the number of pairs appended (0 if session did not qualify).
    pub fn append_session(&self, session: &serde_json::Value) -> usize {
        if !QualityScorer::is_gold_standard(session) {
            return 0;
        }
        let Some(messages) = session["messages"].as_array() else { return 0; };

        let mut appended = 0;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path);
        let Ok(mut file) = file else { return 0; };

        let mut last_user: Option<&str> = None;
        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("");
            let text = msg["content"].as_str().unwrap_or("").trim();
            if text.is_empty() { continue; }
            match role {
                "user" | "User" => last_user = Some(text),
                "assistant" | "Assistant" => {
                    if let Some(instr) = last_user.take() {
                        let entry = FinetuneEntry {
                            instruction: instr.to_string(),
                            input: String::new(),
                            output: text.to_string(),
                        };
                        if let Ok(line) = serde_json::to_string(&entry) {
                            let _ = writeln!(file, "{line}");
                            appended += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        appended
    }

    /// Count the number of entries currently in the store.
    #[must_use]
    pub fn count(&self) -> usize {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    }

    /// Stats about the current store state.
    #[must_use]
    pub fn stats(&self) -> GoldStandardStats {
        let size_bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        GoldStandardStats {
            total_entries: self.count(),
            dataset_path: self.path.to_string_lossy().to_string(),
            size_bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// Flash Distillation (Phase 11)
// ---------------------------------------------------------------------------

pub struct FlashDistiller;

impl FlashDistiller {
    /// Condensed a full session into a set of 'Expert Traces' for the semantic cache.
    /// Focuses only on successful tool sequences and their reasoning.
    pub fn distill_session(session_json: &serde_json::Value) -> Option<String> {
        let success = session_json["success"].as_bool().unwrap_or(false);
        if !success { return None; }

        let messages = session_json["messages"].as_array()?;
        let mut trace = String::new();
        trace.push_str("Expert execution pattern found:\n");

        for msg in messages {
            let role = msg["role"].as_str().unwrap_or("");
            if role == "Assistant" || role == "assistant" {
                if let Some(content) = msg["content"].as_str() {
                    // Extract reasoning (before tool call)
                    if let Some(reasoning) = content.split("```").next() {
                        let trimmed = reasoning.trim();
                        if !trimmed.is_empty() {
                            trace.push_str(&format!("  [Reasoning] {}\n", trimmed));
                        }
                    }
                }
                if let Some(tools) = msg["tool_calls"].as_array() {
                    for tool in tools {
                        let name = tool["function"]["name"].as_str().unwrap_or("unknown");
                        trace.push_str(&format!("  [Tool] {}\n", name));
                    }
                }
            }
        }

        if trace.len() > 30 {
            Some(trace)
        } else {
            None
        }
    }
}

pub struct FlashCompactor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompactionLevel {
    Raw,      // Full message history
    Summary,  // Semantic intent summary
    Sequence, // Tool chain only
    Logic,    // Compressed reasoning
    Flash,    // Minimal context injection (Golden Trace)
}

impl FlashCompactor {
    pub fn distill_to_level(session_json: &serde_json::Value, level: CompactionLevel) -> Option<String> {
        let success = session_json["success"].as_bool().unwrap_or(false);
        if !success { return None; }

        match level {
            CompactionLevel::Raw => Some(serde_json::to_string_pretty(&session_json["messages"]).ok()?),
            CompactionLevel::Summary => {
                session_json["messages"][0]["content"].as_str().map(|s| format!("Task Intent: {}", s))
            }
            CompactionLevel::Sequence => {
                let messages = session_json["messages"].as_array()?;
                let mut sequence = Vec::new();
                for msg in messages {
                    if let Some(content) = msg["content"].as_str() {
                        if content.contains("```") {
                            sequence.push(content.split("```").nth(1).unwrap_or("").trim());
                        }
                    }
                }
                if sequence.is_empty() { None } else { Some(sequence.join("\n-> ")) }
            }
            CompactionLevel::Logic => {
                let messages = session_json["messages"].as_array()?;
                let mut logic = String::new();
                for msg in messages {
                    if msg["role"] == "Assistant" || msg["role"] == "assistant" {
                        if let Some(content) = msg["content"].as_str() {
                            let reasoning = content.split("```").next().unwrap_or("").trim();
                            if !reasoning.is_empty() {
                                logic.push_str(&format!("- {}\n", reasoning));
                            }
                        }
                    }
                }
                if logic.is_empty() { None } else { Some(logic) }
            }
            CompactionLevel::Flash => {
                let messages = session_json["messages"].as_array()?;
                for msg in messages {
                    if msg["role"] == "Assistant" || msg["role"] == "assistant" {
                        return msg["content"].as_str().map(|s| s.to_string());
                    }
                }
                None
            }
        }
    }
}
