//! Internal messaging bus for inter-agent coordination (Mission Control).
//! Uses an async broadcast channel to enable real-time event sharing across the swarm.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Core event types for inter-agent collaboration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MissionEvent {
    /// A general progress update from an agent.
    StatusUpdate {
        agent_id: String,
        mission_id: String,
        status: String,
        percentage: f32,
    },
    /// A new semantic fact discovered in the codebase.
    Discovery {
        agent_id: String,
        file_path: String,
        summary: String,
    },
    /// A potential conflict detected by an agent (e.g., overlapping edits).
    ConflictAlert {
        agent_id: String,
        file_path: String,
        reason: String,
    },
    /// An agent has completed its assigned task.
    TaskCompleted {
        agent_id: String,
        task_id: String,
        output: Option<String>,
    },
    /// A new visual snapshot captured by an agent.
    VisionUpdate {
        agent_id: String,
        snapshot_id: String,
        thumbnail_url: String,
    },
    /// A multi-agent consensus report has been finalized.
    ConsensusFormed {
        agent_id: String,
        report: intelligence::consensus::ConsensusReport,
    },
    /// A system-level heartbeat to ensure the swarm is healthy.
    Heartbeat {
        timestamp: u64,
    },
}

/// The Mission Control bus manager.
#[derive(Debug, Clone)]
pub struct MissionControl {
    tx: broadcast::Sender<MissionEvent>,
}

impl MissionControl {
    /// Create a new Mission Control bus.
    #[must_use] pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast an event to all subscribers.
    pub fn broadcast(&self, event: MissionEvent) -> Result<usize, String> {
        self.tx
            .send(event)
            .map_err(|e| format!("Failed to broadcast event: {e}"))
    }

    /// Subscribe to the mission event stream.
    #[must_use] pub fn subscribe(&self) -> broadcast::Receiver<MissionEvent> {
        self.tx.subscribe()
    }
}

/// Thread-safe wrapper for the Mission Control bus.
pub type ArcMissionControl = Arc<MissionControl>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mission_bus_round_trip() {
        let bus = MissionControl::new(10);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let event = MissionEvent::StatusUpdate {
            agent_id: "agent-1".to_string(),
            mission_id: "m-1".to_string(),
            status: "Initializing...".to_string(),
            percentage: 10.0,
        };

        bus.broadcast(event).unwrap();

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        if let MissionEvent::StatusUpdate { agent_id, .. } = e1 {
            assert_eq!(agent_id, "agent-1");
        } else {
            panic!("Wrong event type");
        }
        
        if let MissionEvent::StatusUpdate { percentage, .. } = e2 {
            assert_eq!(percentage, 10.0);
        } else {
            panic!("Wrong event type");
        }
    }
}
