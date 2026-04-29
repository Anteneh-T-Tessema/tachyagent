//! Sovereign Reputation Score — meritocratic trust system for autonomous agents.

use crate::governance::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    pub success_rate: f32,     // 0.0 to 1.0
    pub token_efficiency: f32, // 0.0 to 1.0
    pub mission_count: usize,
    pub last_failure_days: f32, // Decaying penalty
    pub human_rating: f32,      // 0.0 to 5.0 scale
}

impl Default for ReputationScore {
    fn default() -> Self {
        Self::new()
    }
}

impl ReputationScore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            success_rate: 1.0,
            token_efficiency: 1.0,
            mission_count: 0,
            last_failure_days: 30.0,
            human_rating: 4.0,
        }
    }

    #[must_use]
    pub fn calculate_trust_index(&self) -> f32 {
        let success_weight = 0.5;
        let efficiency_weight = 0.2;
        let rating_weight = 0.3;

        let rating_normalized = self.human_rating / 5.0;

        (self.success_rate * success_weight)
            + (self.token_efficiency * efficiency_weight)
            + (rating_normalized * rating_weight)
    }
}

pub struct TrustElevator;

impl TrustElevator {
    /// Dynamically adjust `PermissionMode` based on `ReputationScore`.
    #[must_use]
    pub fn evaluate_autonomy(score: &ReputationScore) -> PermissionMode {
        let trust = score.calculate_trust_index();
        let mission_threshold = 5;

        if score.mission_count < mission_threshold {
            return PermissionMode::Restricted;
        }

        if trust > 0.95 {
            PermissionMode::SovereignAuto
        } else if trust > 0.85 {
            PermissionMode::AcceptEdits
        } else if trust > 0.70 {
            PermissionMode::Restricted
        } else {
            PermissionMode::PlanOnly
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAuditCard {
    pub agent_id: String,
    pub reputation: ReputationScore,
    pub trust_level: PermissionMode,
    pub history: Vec<MissionOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionOutcome {
    pub mission_id: String,
    pub success: bool,
    pub reward: f32,
    pub ts: u64,
}
