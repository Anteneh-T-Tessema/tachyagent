//! Sovereign Infrastructure — decentralized compute orchestration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeNode {
    pub id: String,
    pub provider: String, // e.g., "Akash", "Render"
    pub specs: NodeSpecs,
    pub status: NodeStatus,
    pub cost_per_hour: f64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpecs {
    pub cpu_cores: u32,
    pub gpu_model: Option<String>,
    pub ram_gb: u32,
    pub storage_gb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Provisioning,
    Active,
    Terminating,
    Offline,
}

pub struct ResourceOrchestrator {
    pub max_burn_rate: f64, // Max $ per hour
}

impl ResourceOrchestrator {
    pub fn new(max_burn_rate: f64) -> Self {
        Self { max_burn_rate }
    }

    /// Provision a new compute node based on mission requirements.
    pub fn provision_node(&self, requirements: &NodeSpecs) -> Result<ComputeNode, String> {
        // Mock decentralized provisioning logic
        let cost = 0.45; // $0.45/hr for a mid-range node
        
        if cost > self.max_burn_rate {
            return Err(format!("Provisioning rejected: Node cost (${:.2}/hr) exceeds max burn rate (${:.2}/hr)", cost, self.max_burn_rate));
        }

        Ok(ComputeNode {
            id: format!("node-{}", uuid::Uuid::new_v4().to_string().chars().take(8).collect::<String>()),
            provider: "Akash".to_string(),
            specs: requirements.clone(),
            status: NodeStatus::Provisioning,
            cost_per_hour: cost,
            expires_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600,
        })
    }
}

pub struct CostEstimator;

impl CostEstimator {
    pub fn estimate_mission_cost(priority: f32, complexity: f32) -> f64 {
        // Simple heuristic for compute requirements
        f64::from((priority * 0.5) + (complexity * 2.0))
    }
}
