//! Sovereign Governance & Trust Spectrum (Phase 15)
//! Based on the "Graduated Trust Spectrum" principle from MBZUAI Research.

use serde::{Deserialize, Serialize};

pub mod dao;
pub use dao::{ConsensusEngine, GovernanceProposal, ProposalStatus, SovereignCharter, VoteRecord};

/// The Graduated Trust Spectrum: 7 levels of autonomous permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// User approves all plans before execution. [cite: 407]
    PlanOnly = 0,
    /// Standard interactive use; most operations require user approval. [cite: 408]
    Restricted = 1,
    /// Edits within the working directory and certain filesystem shell commands are auto-approved. [cite: 409]
    AcceptEdits = 2,
    /// An ML-based classifier evaluates requests based on reward scores. [cite: 411]
    SovereignAuto = 3,
    /// No prompting, but deny rules (safety hooks) are still enforced. [cite: 412]
    DontAsk = 4,
    /// Skips most permission prompts, but safety-critical checks apply. [cite: 413]
    Bypass = 5,
    /// Internal-only mode for subagent permission escalation. [cite: 414]
    Bubble = 6,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Restricted
    }
}

/// A policy decision based on the trust spectrum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny,
    Escalate, // Ask the human
}

impl PermissionMode {
    /// Evaluate a proposed action against the trust spectrum.
    #[must_use]
    pub fn evaluate(
        &self,
        is_read_only: bool,
        is_in_workspace: bool,
        reward_score: f32,
    ) -> PolicyDecision {
        match self {
            Self::PlanOnly => PolicyDecision::Escalate,
            Self::Restricted => {
                if is_read_only {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Escalate
                }
            }
            Self::AcceptEdits => {
                if is_read_only || is_in_workspace {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Escalate
                }
            }
            Self::SovereignAuto => {
                if reward_score > 0.90 {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Escalate
                }
            }
            Self::DontAsk | Self::Bypass | Self::Bubble => PolicyDecision::Allow,
        }
    }
}
