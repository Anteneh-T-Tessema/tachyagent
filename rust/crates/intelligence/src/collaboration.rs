//! Tools for inter-agent collaboration and Mission Control updates.

use serde::{Deserialize, Serialize};

/// Input for the `broadcast_mission_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastStatusInput {
    /// Percentage complete for the current sub-task (0.0 to 100.0).
    pub percentage: f32,
    /// Detailed status message (e.g., "Refactoring modules/auth...").
    pub status: String,
    /// Whether this is a critical discovery that others should know about.
    #[serde(default)]
    pub discovery: Option<String>,
}

/// Output for the `broadcast_mission_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastStatusResult {
    /// Whether the broadcast was successful.
    pub success: bool,
    /// Number of active agents notified.
    pub listeners: usize,
}

/// Input for the `get_mission_feed` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMissionFeedInput {
    /// Max number of recent events to retrieve.
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

/// A single event from the mission feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionFeedEvent {
    pub agent_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub timestamp: u64,
}

/// Result for the `get_mission_feed` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMissionFeedResult {
    /// List of recent events from other agents in the swarm.
    pub events: Vec<MissionFeedEvent>,
}

/// Specification for the collaboration tools.
#[must_use]
pub fn collaboration_tool_specs() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "broadcast_mission_status",
            "description": "Broadcasts a progress update or a new discovery to all other agents in the current swarm. Use this when you finish a critical file or find an unexpected pattern.",
            "parameters": {
                "type": "object",
                "properties": {
                    "percentage": { "type": "number", "description": "Progress percentage (0-100)" },
                    "status": { "type": "string", "description": "Description of current work" },
                    "discovery": { "type": "string", "description": "Optional: New architectural pattern or problem discovered" }
                },
                "required": ["percentage", "status"]
            }
        }),
        serde_json::json!({
            "name": "get_mission_feed",
            "description": "Retrieves the recent activity log of all agents in the swarm. Use this to ensure you are not duplicating work or to check if another agent has found a blocker.",
            "parameters": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max events to return" }
                }
            }
        }),
    ]
}
