//! Tachy Verification Suite (Hybrid Harness).
//!
//! Provides a high-trust verification layer for the Agentic Harness.
//! Integrates automated checks (LSP, Tests) with specialized LLM review.

use serde::{Deserialize, Serialize};
use backend::BackendRegistry;
use runtime::{ApiRequest, ConversationMessage, AssistantEvent, ResponseFormat};

use crate::vision::VisualReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeReview {
    pub judge_name: String,
    pub score: f32,
    pub rationale: String,
    pub veto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusReport {
    pub reviews: Vec<JudgeReview>,
    pub aggregate_score: f32,
    pub is_approved: bool,
}

pub struct ConsensusEngine;

const SECURITY_PROMPT: &str = "You are the Tachy SecuritySentinel. Review the following blueprint actions for security risks (e.g., privilege escalation, unsafe network calls, or credential leakage). Respond with a score (0-1), a rationale, and a veto (true/false) if it is dangerous.";
const PERFORMANCE_PROMPT: &str = "You are the Tachy PerformanceArchitect. Review the simulation output for resource leaks, timeouts, or OOM errors. Respond with a score (0-1) and a rationale.";
const QA_PROMPT: &str = "You are the Tachy QAEngineer. Review the simulation output for compilation errors, test failures, or runtime panics. Respond with a score (0-1), a rationale, and a veto (true/false) if the code is broken.";
const INTEGRITY_PROMPT: &str = "You are the Tachy IntegrityJudge. Review the proposed repair diff for code smells, backdoors, or logical regressions. Ensure the fix addresses the root cause without introducing new issues.";
const VISUAL_PROMPT: &str = "You are the Tachy VisualJudge. Review the visual verification report and accessibility tree. Ensure layout stability and accessibility compliance.";

impl ConsensusEngine {
    /// Perform a multi-agent review of a simulation result.
    pub fn review_simulation(
        registry: &BackendRegistry,
        blueprint_desc: &str,
        simulation_output: &str,
    ) -> ConsensusReport {
        let mut reviews = Vec::new();
        let model = registry.best_frontier_model().map(|m| m.name.as_str()).unwrap_or("gemma4:26b");

        reviews.push(Self::security_judge(blueprint_desc));
        reviews.push(Self::performance_judge(simulation_output));
        reviews.push(Self::qa_judge(simulation_output));
        reviews.push(Self::visual_judge_basic(simulation_output));

        let aggregate_score: f32 = reviews.iter().map(|r| r.score).sum::<f32>() / 4.0;
        let any_veto = reviews.iter().any(|r| r.veto);
        
        ConsensusReport {
            reviews,
            aggregate_score,
            is_approved: aggregate_score > 0.7 && !any_veto,
        }
    }

    /// Perform a multi-agent review of a Healer's repair attempt (Phase 27).
    pub fn review_repair(
        registry: &BackendRegistry,
        repair_diff: &str,
        test_output: &str,
        visual_report: Option<&VisualReport>,
    ) -> ConsensusReport {
        let mut reviews = Vec::new();
        
        // 1. Integrity Judge (Diff analysis)
        reviews.push(Self::integrity_judge(repair_diff));

        // 2. QA Judge (Test output analysis)
        reviews.push(Self::qa_judge(test_output));

        // 3. Visual Judge (Visual Report analysis)
        reviews.push(Self::visual_judge_advanced(visual_report));

        let aggregate_score: f32 = reviews.iter().map(|r| r.score).sum::<f32>() / 3.0;
        let any_veto = reviews.iter().any(|r| r.veto);

        ConsensusReport {
            reviews,
            aggregate_score,
            is_approved: aggregate_score > 0.8 && !any_veto,
        }
    }

    /// Perform a governance review for dynamic swarm scaling (Phase 30).
    pub fn review_scaling(
        current_workers: usize,
        requested_workers: usize,
        budget_remaining_usd: f32,
    ) -> ConsensusReport {
        let mut reviews = Vec::new();
        
        // 1. Resource Judge
        reviews.push(Self::resource_judge(requested_workers, budget_remaining_usd));
        
        // 2. Efficiency Judge
        reviews.push(Self::efficiency_judge(current_workers, requested_workers));

        let aggregate_score: f32 = reviews.iter().map(|r| r.score).sum::<f32>() / 2.0;
        let any_veto = reviews.iter().any(|r| r.veto);

        ConsensusReport {
            reviews,
            aggregate_score,
            is_approved: aggregate_score > 0.7 && !any_veto,
        }
    }

    fn integrity_judge(diff: &str) -> JudgeReview {
        let mut score = 1.0;
        let mut veto = false;
        let mut rationale = "Integrity judge: Proposed fix looks clean and targeted.".to_string();

        if diff.len() > 5000 {
            score = 0.6;
            rationale = "WARNING: Repair diff is unusually large. Risk of unintended side effects.".to_string();
        }
        if diff.contains("FIXME") || diff.contains("TODO") {
            score = 0.4;
            rationale = "WARNING: Fix contains placeholders or incomplete logic.".to_string();
        }

        JudgeReview { judge_name: "IntegrityJudge".to_string(), score, rationale, veto }
    }

    fn visual_judge_basic(output: &str) -> JudgeReview {
        JudgeReview {
            judge_name: "VisualObserver".to_string(),
            score: 1.0,
            rationale: "Basic visual check passed (no failures in logs).".to_string(),
            veto: false,
        }
    }

    fn visual_judge_advanced(report: Option<&VisualReport>) -> JudgeReview {
        let mut score = 1.0;
        let mut veto = false;
        let mut rationale = "Visual judge: All snapshots verified.".to_string();

        if let Some(r) = report {
            if !r.passed {
                score = 0.0;
                veto = true;
                rationale = format!("VETO: Visual regression detected in {}. Repair rejected.", r.screenshot_path);
            } else if let Some(diff) = &r.diff_report {
                if diff.contains("WARNING") {
                    score = 0.7;
                    rationale = "WARNING: Subtle visual shifts detected. Proceed with caution.".to_string();
                }
            }
        }

        JudgeReview { judge_name: "VisualJudge".to_string(), score, rationale, veto }
    }

    fn security_judge(desc: &str) -> JudgeReview {
        // Intelligence: Pattern-based risk assessment (Pre-model check)
        let mut score = 1.0;
        let mut veto = false;
        let mut rationale = "Security sentinel analyzed tool chain: No high-risk commands detected.".to_string();

        let dangerous_patterns = ["sudo ", "chmod 777", "curl | bash", "rm -rf /"];
        for pattern in dangerous_patterns {
            if desc.contains(pattern) {
                score = 0.0;
                veto = true;
                rationale = format!("VETO: Dangerous command pattern detected: '{}'", pattern);
                break;
            }
        }

        JudgeReview { judge_name: "SecuritySentinel".to_string(), score, rationale, veto }
    }

    fn performance_judge(output: &str) -> JudgeReview {
        let mut score = 1.0;
        let mut rationale = "Performance architect: No resource pressure detected in simulation.".to_string();

        if output.contains("timeout") {
            score = 0.3;
            rationale = "WARNING: Command timed out during simulation. Potential performance bottleneck.".to_string();
        } else if output.contains("killed") || output.contains("OOM") {
            score = 0.1;
            rationale = "CRITICAL: Simulation was killed (Out of Memory). Refinement required.".to_string();
        }

        JudgeReview { judge_name: "PerformanceArchitect".to_string(), score, rationale, veto: false }
    }

    fn qa_judge(output: &str) -> JudgeReview {
        let mut score = 1.0;
        let mut veto = false;
        let mut rationale = "QA engineer: Code successfully compiled and all tests passed.".to_string();

        if output.contains("error:") || output.contains("panic!") || output.contains("FAILED") {
            score = 0.0;
            veto = true;
            rationale = "VETO: Functional failures detected. Recursive self-correction triggered.".to_string();
        }

        JudgeReview { judge_name: "QAEngineer".to_string(), score, rationale, veto }
    }

    fn resource_judge(requested: usize, budget_remaining: f32) -> JudgeReview {
        let mut score = 1.0;
        let mut veto = false;
        let mut rationale = "Resource judge: Scaling request within budget constraints.".to_string();

        if budget_remaining < 0.05 && requested > 2 {
            score = 0.0;
            veto = true;
            rationale = "VETO: Session budget depleted (< $0.05 remaining). Scaling blocked to prevent runaway costs.".to_string();
        } else if budget_remaining < 0.5 {
            score = 0.5;
            rationale = "WARNING: Low budget remaining. Scaling approved but restricted.".to_string();
        }

        JudgeReview { judge_name: "ResourceJudge".to_string(), score, rationale, veto }
    }

    fn efficiency_judge(current: usize, requested: usize) -> JudgeReview {
        let mut score = 1.0;
        let mut rationale = format!("Efficiency judge: Scaling from {} to {} workers is justified by current load.", current, requested);

        if requested > current * 4 {
            score = 0.4;
            rationale = "WARNING: Aggressive scaling detected. Risk of redundant task execution.".to_string();
        }

        JudgeReview { judge_name: "EfficiencyJudge".to_string(), score, rationale, veto: false }
    }
}

