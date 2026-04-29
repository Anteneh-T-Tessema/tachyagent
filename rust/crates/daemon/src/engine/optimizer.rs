//! Swarm Optimization Engine — autonomous self-evolution for agent intelligence.

use audit::AuditEventKind;
use intelligence::{ConsensusReport, IntelligenceConfig};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationProposal {
    pub id: String,
    pub template_name: String,
    pub original_prompt: String,
    pub optimized_prompt: String,
    pub rationale: String,
    pub impact_score: f32,
    pub status: OptimizationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OptimizationStatus {
    Pending,
    Approved,
    Rejected,
    Applied,
    RolledBack,
}

pub struct EvolutionManager {
    workspace_root: std::path::PathBuf,
}

impl EvolutionManager {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// Analyze recent audit logs and propose optimizations.
    pub fn analyze_performance(&self, config: &IntelligenceConfig) -> Vec<OptimizationProposal> {
        if !config.optimization.enabled {
            return Vec::new();
        }

        let audit_path = self.workspace_root.join(".tachy").join("audit.jsonl");
        let events = Self::load_recent_events(&audit_path, config.optimization.log_window);

        let mut proposals = Vec::new();

        // Group events by agent/session to find failures
        // For each failure, if it has a consensus report with a low score, analyze it
        for event in events {
            if event.kind == AuditEventKind::GovernanceViolation
                || event.kind == AuditEventKind::SelfRepair
            {
                if let Some(report_val) = &event.consensus_report {
                    if let Ok(report) =
                        serde_json::from_value::<ConsensusReport>(report_val.clone())
                    {
                        if report.aggregate_score < config.optimization.score_threshold {
                            if let Some(proposal) = Self::generate_proposal(&event, &report) {
                                proposals.push(proposal);
                            }
                        }
                    }
                }
            }
        }

        proposals
    }

    fn load_recent_events(path: &Path, window: usize) -> Vec<audit::AuditEvent> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .rev()
            .take(window)
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn generate_proposal(
        event: &audit::AuditEvent,
        report: &ConsensusReport,
    ) -> Option<OptimizationProposal> {
        // Intelligence: In a real implementation, we would call an LLM here to analyze the failure
        // and propose a better system prompt. For Phase 31, we generate a structured proposal
        // that highlights the most critical judge feedback.

        let agent_id = event.agent_id.as_deref().unwrap_or("unknown");
        let critical_feedback = report
            .reviews
            .iter()
            .filter(|r| r.score < 0.5)
            .map(|r| format!("{}: {}", r.judge_name, r.rationale))
            .collect::<Vec<_>>()
            .join("\n");

        if critical_feedback.is_empty() {
            return None;
        }

        Some(OptimizationProposal {
            id: format!("opt-{}", uuid::Uuid::new_v4().simple()),
            template_name: agent_id.to_string(), // Simplified: assuming agent_id matches template
            original_prompt: "[ORIGINAL SYSTEM PROMPT]".to_string(),
            optimized_prompt: format!("[OPTIMIZED PROMPT]\n\nADVISORY: {critical_feedback}\n"),
            rationale: format!(
                "Low consensus score ({:.2}). Critical issues identified by Judges.",
                report.aggregate_score
            ),
            impact_score: 1.0 - report.aggregate_score,
            status: OptimizationStatus::Pending,
        })
    }
}
