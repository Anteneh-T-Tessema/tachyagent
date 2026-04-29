//! Sovereign Governance DAO — autonomous system configuration management.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProposalStatus {
    Pending,
    Passed,
    Rejected,
    Vetoed,
    Executed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceProposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub change_type: String, // e.g., "sentinel_rule", "investment_policy"
    pub payload: serde_json::Value,
    pub votes_yes: usize,
    pub votes_no: usize,
    pub status: ProposalStatus,
    pub creator_agent: String,
    pub deadline: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteRecord {
    pub proposal_id: String,
    pub agent_id: String,
    pub vote: bool,
    pub rationale: String,
}

pub struct ConsensusEngine {
    pub threshold: f32, // e.g., 0.66 for 2/3 majority
}

impl ConsensusEngine {
    #[must_use]
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    pub fn evaluate(
        &self,
        proposal: &mut GovernanceProposal,
        total_eligible_voters: usize,
    ) -> ProposalStatus {
        if proposal.status != ProposalStatus::Pending {
            return proposal.status.clone();
        }

        let total_votes = proposal.votes_yes + proposal.votes_no;
        if total_eligible_voters == 0 {
            return ProposalStatus::Pending;
        }

        let participation = total_votes as f32 / total_eligible_voters as f32;
        let support = if total_votes > 0 {
            proposal.votes_yes as f32 / total_votes as f32
        } else {
            0.0
        };

        if participation >= 0.5 {
            // 50% quorum
            if support >= self.threshold {
                return ProposalStatus::Passed;
            } else if support < (1.0 - self.threshold) {
                return ProposalStatus::Rejected;
            }
        }

        ProposalStatus::Pending
    }
}

pub struct SovereignCharter {
    pub immutable_rules: Vec<String>,
}

impl Default for SovereignCharter {
    fn default() -> Self {
        Self::new()
    }
}

impl SovereignCharter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            immutable_rules: vec![
                "Never disable the Compliance Sentinel.".to_string(),
                "Human Veto is always absolute.".to_string(),
                "Minimum security severity for core updates is CRITICAL.".to_string(),
            ],
        }
    }

    pub fn validate_proposal(&self, proposal: &GovernanceProposal) -> Result<(), String> {
        // Mock validation logic
        if proposal.title.to_lowercase().contains("disable sentinel") {
            return Err("Proposal violates Sovereign Charter: Immutable rule 'Never disable the Compliance Sentinel'.".to_string());
        }
        Ok(())
    }
}
