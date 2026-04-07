//! Fine-tuning dataset extraction and LoRA workflow helpers.
//!
//! Converts Tachy session history into Alpaca-format JSONL suitable for
//! LoRA fine-tuning with Unsloth, Axolotl, or llama.cpp.
//!
//! Also generates:
//! - An Ollama `Modelfile` for packaging a custom-adapted model.
//! - A shell training script (informational; no actual training is run here).

use std::path::Path;

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

/// A collected dataset ready for LoRA training.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FinetuneDataset {
    pub entries: Vec<FinetuneEntry>,
    pub source_sessions: usize,
    pub total_pairs: usize,
}

impl FinetuneDataset {
    /// Extract training pairs from all `.json` session files in `sessions_dir`.
    ///
    /// Each consecutive (user, assistant) turn pair becomes one training example.
    pub fn from_sessions(sessions_dir: &Path) -> Self {
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
    pub fn to_jsonl(&self) -> String {
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
}

// ---------------------------------------------------------------------------
// Modelfile generation
// ---------------------------------------------------------------------------

/// Generate an Ollama `Modelfile` that packages a LoRA-adapted model.
///
/// # Arguments
/// * `base_model`    - e.g. `"mistral:7b"` or `"gemma:7b"`
/// * `adapter_path`  - path to the `.gguf` LoRA adapter produced by training
/// * `system_prompt` - custom system prompt baked into the model
pub fn generate_modelfile(base_model: &str, adapter_path: &str, system_prompt: &str) -> String {
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

/// Generate a shell script for running LoRA fine-tuning with Unsloth.
///
/// This is informational output only — no training is performed by Tachy.
/// The script is written to disk so the user can inspect, modify, and run it.
pub fn generate_training_script(model_id: &str, dataset_path: &str, output_dir: &str) -> String {
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
        let ds = FinetuneDataset::from_sessions(&dir);
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
        let ds = FinetuneDataset::from_sessions(&dir);
        assert_eq!(ds.total_pairs, 2);
        assert_eq!(ds.entries[0].instruction, "What is Rust?");
        assert_eq!(ds.entries[0].output, "Rust is a systems language.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skips_non_json_files() {
        let dir = tmp_dir("skip-non-json");
        std::fs::write(dir.join("notes.txt"), "not a session").unwrap();
        let ds = FinetuneDataset::from_sessions(&dir);
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
        let lines: Vec<&str> = ds.to_jsonl().lines().collect();
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
}
