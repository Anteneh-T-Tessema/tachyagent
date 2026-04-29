use serde::{Deserialize, Serialize};

/// Information about a peer node in the sovereign swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub url: String,
    pub status: PeerStatus,
    pub capabilities: Vec<String>,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Online,
    Offline,
    Busy,
}

/// A task delegated from one node to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedTask {
    pub sender_id: String,
    pub session_id: String,
    pub task_id: String,
    pub template_name: String,
    pub instruction: String,
    /// Context files relevant to this step.
    pub context: Vec<FileContext>,
    /// Cryptographic signature of the task hash.
    pub signature: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContext {
    pub path: String,
    pub content: String,
}

/// Result of a delegated task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub modified_files: Vec<FileContext>,
    pub signature: String,
    pub public_key: String,
}
