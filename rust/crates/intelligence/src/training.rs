//! Sovereign Intelligence — autonomous self-improvement via fine-tuning.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::finetune::{TrainingJob, TrainingStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingTrace {
    pub session_id: String,
    pub steps: Vec<TraceStep>,
    pub reward_score: f32, // 0.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub prompt: String,
    pub response: String,
    pub outcome: String,
}

pub struct TraceExtractor;

impl TraceExtractor {
    /// Extract "Gold Standard" traces (reward > 0.9) from session logs.
    #[must_use]
    pub fn extract_gold_traces(all_traces: Vec<TrainingTrace>) -> Vec<TrainingTrace> {
        all_traces
            .into_iter()
            .filter(|t| t.reward_score > 0.9)
            .collect()
    }
}

pub struct DatasetGenerator;

impl DatasetGenerator {
    /// Convert traces into a fine-tuning dataset (JSONL format).
    #[must_use]
    pub fn generate_jsonl(traces: &[TrainingTrace]) -> String {
        let mut dataset = String::new();
        for trace in traces {
            for step in &trace.steps {
                let entry = serde_json::json!({
                    "instruction": step.prompt,
                    "input": "",
                    "output": step.response
                });
                dataset.push_str(&entry.to_string());
                dataset.push('\n');
            }
        }
        dataset
    }
}

pub struct TrainerOrchestrator;

impl TrainerOrchestrator {
    /// Trigger a remote fine-tuning job on a high-compute node.
    pub fn start_tuning_job(&self, _dataset: &str, _expert_name: &str) -> Result<String, String> {
        // Mock training orchestration logic
        Ok(format!(
            "job-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpertAdapter {
    pub id: String,
    pub base_model: String,
    pub domain: String,
    /// Performance lift over the base model measured by A/B evaluation (0.0 = unknown).
    pub lift_score: f32,
    pub status: AdapterStatus,
    /// Path to the GGUF / `LoRA` adapter file on disk (empty = remote/pending).
    #[serde(default)]
    pub adapter_path: String,
    /// Timestamp when this adapter was registered (Unix seconds).
    #[serde(default)]
    pub registered_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterStatus {
    Training,
    Ready,
    Active,
    Deprecated,
}

// ---------------------------------------------------------------------------
// Adapter Registry — persistent CRUD for Expert Adapters
// ---------------------------------------------------------------------------

/// Durable registry of Expert Adapters backed by `.tachy/adapters.json`.
///
/// The registry is the single source of truth for adapter lifecycle:
/// `Training → Ready → Active`.  Only one adapter per domain may be `Active`
/// at a time; activating a new one automatically demotes the previous one to
/// `Ready`.
pub struct AdapterRegistry {
    path: PathBuf,
    adapters: Vec<ExpertAdapter>,
}

impl AdapterRegistry {
    /// Load registry from `path` (creates an empty registry if the file does
    /// not exist or cannot be parsed).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let adapters = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, adapters }
    }

    /// Persist the registry to disk atomically (write-then-rename).
    pub fn save(&self) -> Result<(), String> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.adapters).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// Register a new adapter. Returns its assigned `id`.
    pub fn register(&mut self, base_model: &str, domain: &str, adapter_path: &str) -> String {
        let id = format!(
            "adapter-{}-{}",
            domain.to_lowercase().replace(' ', "-"),
            now_epoch()
        );
        self.adapters.push(ExpertAdapter {
            id: id.clone(),
            base_model: base_model.to_string(),
            domain: domain.to_string(),
            lift_score: 0.0,
            status: AdapterStatus::Ready,
            adapter_path: adapter_path.to_string(),
            registered_at: now_epoch(),
        });
        let _ = self.save();
        id
    }

    /// Activate an adapter by id. Demotes any existing Active adapter in the
    /// same domain to `Ready`. Returns `Err` if id is not found.
    pub fn activate(&mut self, id: &str) -> Result<(), String> {
        let domain = self
            .adapters
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.domain.clone())
            .ok_or_else(|| format!("adapter not found: {id}"))?;

        // Demote any currently-active adapter in the same domain
        for a in &mut self.adapters {
            if a.domain == domain && a.status == AdapterStatus::Active && a.id != id {
                a.status = AdapterStatus::Ready;
            }
        }

        // Activate the target
        self.adapters
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("adapter not found: {id}"))?
            .status = AdapterStatus::Active;

        self.save()
    }

    /// Update the lift score for an adapter after an A/B evaluation.
    pub fn update_lift_score(&mut self, id: &str, lift: f32) -> Result<(), String> {
        self.adapters
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("adapter not found: {id}"))?
            .lift_score = lift;
        self.save()
    }

    /// Mark a training-in-progress adapter as Ready.
    pub fn mark_ready(&mut self, id: &str) -> Result<(), String> {
        let a = self
            .adapters
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("adapter not found: {id}"))?;
        a.status = AdapterStatus::Ready;
        self.save()
    }

    #[must_use]
    pub fn list(&self) -> &[ExpertAdapter] {
        &self.adapters
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ExpertAdapter> {
        self.adapters.iter().find(|a| a.id == id)
    }

    /// Return the currently Active adapter for a given domain, if any.
    #[must_use]
    pub fn active_for_domain(&self, domain: &str) -> Option<&ExpertAdapter> {
        self.adapters
            .iter()
            .find(|a| a.domain == domain && a.status == AdapterStatus::Active)
    }

    /// Sync external adapters into the registry (used by `DaemonState` migration).
    pub fn sync_from_vec(&mut self, adapters: &[ExpertAdapter]) {
        for a in adapters {
            if !self.adapters.iter().any(|r| r.id == a.id) {
                self.adapters.push(a.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// A/B test result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTestResult {
    pub adapter_a: String,
    pub adapter_b: String,
    /// Proxy quality score for adapter A (0.0..1.0).
    pub score_a: f32,
    /// Proxy quality score for adapter B (0.0..1.0).
    pub score_b: f32,
    /// The winning adapter id.
    pub winner: String,
    /// Lift of winner over loser (0.0..1.0).
    pub lift: f32,
}

// ---------------------------------------------------------------------------
// Fine-tuning Bridge — closed-loop training orchestration
// ---------------------------------------------------------------------------

/// Bridges the Gold Standard dataset to the `LoRA` fine-tuning pipeline.
///
/// In production, `trigger_from_dataset` would submit to a remote training
/// cluster.  Here it is mock-wired so the daemon can track jobs without
/// requiring GPU hardware.
pub struct FineTuningBridge;

impl FineTuningBridge {
    /// Trigger a training job from the gold standard dataset at `dataset_path`.
    /// Returns the new `TrainingJob` on success or an error string if the
    /// dataset is missing/empty.
    pub fn trigger_from_dataset(
        dataset_path: &Path,
        base_model: &str,
        adapter_name: &str,
    ) -> Result<TrainingJob, String> {
        if !dataset_path.exists() {
            return Err(format!("dataset not found: {}", dataset_path.display()));
        }
        let content = std::fs::read_to_string(dataset_path).map_err(|e| e.to_string())?;
        let entry_count = content.lines().filter(|l| !l.trim().is_empty()).count();
        if entry_count == 0 {
            return Err("dataset is empty — collect more Gold Standard sessions first".into());
        }

        let job = TrainingJob {
            id: format!("job-{}", now_epoch()),
            status: TrainingStatus::Queued,
            adapter_name: adapter_name.to_string(),
            start_time: now_epoch(),
            end_time: None,
            error: None,
        };
        // In a real implementation, we would POST to a remote trainer here.
        // For now, log the intent so operators can pick it up.
        eprintln!(
            "[finetune] training job {} queued: model={} entries={} adapter={}",
            job.id, base_model, entry_count, adapter_name
        );
        Ok(job)
    }

    /// Simple A/B evaluation: run `prompt` through two model names (resolved
    /// from the registry) and compare response quality by a proxy metric
    /// (response length × coherence heuristic).
    ///
    /// Since no actual LLM is called here, we generate mock scores based on
    /// the adapter's registered `lift_score`.  Real implementations would call
    /// the model backend and run a judge.
    #[must_use]
    pub fn ab_test(adapter_a: &ExpertAdapter, adapter_b: &ExpertAdapter) -> ABTestResult {
        // Proxy scores: lift_score contribution + baseline noise
        let score_a = (0.5 + adapter_a.lift_score * 0.5).min(1.0);
        let score_b = (0.5 + adapter_b.lift_score * 0.5).min(1.0);
        let (winner, lift) = if score_a >= score_b {
            (adapter_a.id.clone(), score_a - score_b)
        } else {
            (adapter_b.id.clone(), score_b - score_a)
        };
        ABTestResult {
            adapter_a: adapter_a.id.clone(),
            adapter_b: adapter_b.id.clone(),
            score_a,
            score_b,
            winner,
            lift,
        }
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
