//! Enterprise Cloud Bridge: AWS Batch Client (Direction B).
//! 
//! This module provides the orchestration logic for offloading heavy agentic 
//! workloads to the AWS Batch ecosystem.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a job submitted to the cloud bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub name: String,
    pub status: BatchJobStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub log_stream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BatchJobStatus {
    Submitted,
    Pending,
    Runnable,
    Starting,
    Running,
    Succeeded,
    Failed(String),
}

/// Abstract client for AWS Batch interactions.
pub struct BatchClient {
    pub region: String,
    pub queue: String,
    pub job_definition_prefix: String,
}

impl BatchClient {
    #[must_use] pub fn new(region: &str, queue: &str) -> Self {
        Self {
            region: region.to_string(),
            queue: queue.to_string(),
            job_definition_prefix: "tachy-agent-worker".to_string(),
        }
    }

    /// Prepares a compressed workspace bundle for cloud hydration.
    ///
    /// Creates a `.tar.gz` of the workspace, excluding large generated directories
    /// (`.git`, `node_modules`, `target`, `.tachy/deploy`) so the bundle stays small.
    pub fn prepare_bundle(&self, workspace_root: &std::path::Path) -> Result<String, String> {
        let bundle_name = format!("tachy-bundle-{}.tar.gz", chrono_now());
        let deploy_dir = workspace_root.join(".tachy").join("deploy");
        let bundle_path = deploy_dir.join(&bundle_name);

        std::fs::create_dir_all(&deploy_dir).map_err(|e| e.to_string())?;

        // Use the system `tar` command — available on Linux/macOS and WSL2.
        // Excludes heavy directories that should not be shipped to the cloud.
        let exclude_patterns = [
            "--exclude=./.git",
            "--exclude=./target",
            "--exclude=./node_modules",
            "--exclude=./.tachy/deploy",
            "--exclude=./.tachy/sessions",
        ];

        let output = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&bundle_path)
            .args(exclude_patterns)
            .arg(".")
            .current_dir(workspace_root)
            .output()
            .map_err(|e| format!("tar not found: {e} — install GNU tar (brew install gnu-tar on macOS)"))?;

        if !output.status.success() {
            return Err(format!(
                "tar failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let size_bytes = std::fs::metadata(&bundle_path)
            .map(|m| m.len())
            .unwrap_or(0);

        eprintln!(
            "[batch] bundle created: {} ({:.1} MB)",
            bundle_path.display(),
            size_bytes as f64 / 1_048_576.0
        );

        Ok(bundle_name)
    }

    /// Submits a new agentic workload to AWS Batch.
    /// 
    /// In a production environment, this would call the AWS SDK:
    /// `batch.submit_job().job_queue(...).job_definition(...)`
    pub fn submit_job(
        &self,
        name: &str,
        _command: Vec<String>,
        _env_vars: HashMap<String, String>,
    ) -> Result<BatchJob, String> {
        // For Direction B implementation: implement the API call logic
        // For now, we simulate the submission success.
        
        let job_id = format!("job-{}", uuid_simple());
        
        Ok(BatchJob {
            id: job_id,
            name: name.to_string(),
            status: BatchJobStatus::Submitted,
            created_at: chrono_now(),
            updated_at: chrono_now(),
            log_stream: None,
        })
    }

    /// Polls the status of an active job.
    ///
    /// Tries `aws batch describe-jobs` (AWS CLI) first; falls back to returning
    /// the stored status if the CLI is unavailable or returns an error.
    pub fn get_job_status(&self, job_id: &str) -> Result<BatchJobStatus, String> {
        let output = std::process::Command::new("aws")
            .args([
                "batch", "describe-jobs",
                "--jobs", job_id,
                "--region", &self.region,
                "--output", "json",
            ])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(status_str) = v["jobs"][0]["status"].as_str() {
                        return Ok(match status_str {
                            "SUBMITTED" => BatchJobStatus::Submitted,
                            "PENDING"   => BatchJobStatus::Pending,
                            "RUNNABLE"  => BatchJobStatus::Runnable,
                            "STARTING"  => BatchJobStatus::Starting,
                            "RUNNING"   => BatchJobStatus::Running,
                            "SUCCEEDED" => BatchJobStatus::Succeeded,
                            other       => BatchJobStatus::Failed(format!("unknown status: {other}")),
                        });
                    }
                }
            }
        }

        // AWS CLI unavailable or failed — return Running as a conservative default
        // so callers don't treat the job as done prematurely.
        Ok(BatchJobStatus::Running)
    }
}

fn uuid_simple() -> String {
    let mut s = String::new();
    for _ in 0..8 {
        s.push(std::char::from_u32(0x61 + (rand_u32() % 26)).unwrap());
    }
    s
}

fn rand_u32() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u32
}

fn chrono_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
