//! Reward Engine for Reinforcement Learning from Execution Feedback (RLEF).
//!
//! Provides a quantitative framework for evaluating agent trajectories based on:
//! - Execution success (tests + LSP)
//! - Code quality (complexity + warnings)
//! - Efficiency (tokens + latency)
//! - Verifiability (Forensic Logic Layer)

use serde::{Deserialize, Serialize};
use crate::edit_test_fix::{DiagnosticResult, TestResult};

/// A multi-dimensional breakdown of an agent's performance for a single step or session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RewardScore {
    /// Base score for achieving the goal (0.0 to 10.0).
    pub success: f32,
    /// Penalty for low-quality code (negative).
    pub quality_penalty: f32,
    /// Penalty for inefficiency (negative).
    pub efficiency_penalty: f32,
    /// Bonus for forensic verifiability (positive).
    pub forensic_bonus: f32,
    /// Total aggregate reward.
    pub total: f32,
}

/// Configuration for the reward model's weights and thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardConfig {
    pub success_weight: f32,
    pub error_penalty: f32,
    pub warning_penalty: f32,
    pub token_penalty_per_k: f32,
    pub latency_penalty_per_sec: f32,
    pub forensic_bonus_weight: f32,
}

impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            success_weight: 10.0,
            error_penalty: -5.0,
            warning_penalty: -0.5,
            token_penalty_per_k: -0.01,
            latency_penalty_per_sec: -0.05,
            forensic_bonus_weight: 2.0,
        }
    }
}

pub struct RewardEngine {
    config: RewardConfig,
}

impl RewardEngine {
    #[must_use] pub fn new(config: RewardConfig) -> Self {
        Self { config }
    }

    /// Calculate the reward for a single execution step.
    #[must_use] pub fn calculate_step_reward(
        &self,
        test_result: Option<&TestResult>,
        diag_result: Option<&DiagnosticResult>,
        tokens_used: u32,
        duration_ms: u64,
        is_verifiable: bool,
    ) -> RewardScore {
        let mut score = RewardScore::default();

        // 1. Success Reward
        if let Some(tr) = test_result {
            if tr.exit_code == 0 {
                score.success = self.config.success_weight;
            } else {
                score.success = 0.0;
            }
        }

        // 2. Quality Penalties (LSP)
        if let Some(dr) = diag_result {
            score.quality_penalty += dr.error_count as f32 * self.config.error_penalty;
            score.quality_penalty += dr.warning_count as f32 * self.config.warning_penalty;
        }

        // 3. Efficiency Penalties
        score.efficiency_penalty += (tokens_used as f32 / 1000.0) * self.config.token_penalty_per_k;
        score.efficiency_penalty += (duration_ms as f32 / 1000.0) * self.config.latency_penalty_per_sec;

        // 4. Forensic Bonus
        if is_verifiable {
            score.forensic_bonus = self.config.forensic_bonus_weight;
        }

        score.total = score.success + score.quality_penalty + score.efficiency_penalty + score.forensic_bonus;
        score
    }

    /// Evaluate a full trajectory (session history).
    #[must_use] pub fn evaluate_trajectory(&self, rewards: &[RewardScore]) -> f32 {
        if rewards.is_empty() { return 0.0; }
        // For now, return the mean reward
        let total: f32 = rewards.iter().map(|r| r.total).sum();
        total / rewards.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_test_fix::DiagnosticResult;

    #[test]
    fn lsp_errors_tank_the_score() {
        let engine = RewardEngine::new(RewardConfig::default());
        let dr = DiagnosticResult { files_checked: 1, error_count: 2, warning_count: 0, diagnostics: Vec::new() };
        
        let score = engine.calculate_step_reward(None, Some(&dr), 100, 100, false);
        
        assert!(score.quality_penalty <= -10.0);
        assert!(score.total < 0.0);
    }

    #[test]
    fn perfect_execution_gets_high_reward() {
        let engine = RewardEngine::new(RewardConfig::default());
        let tr = TestResult { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let dr = DiagnosticResult { files_checked: 1, error_count: 0, warning_count: 0, diagnostics: Vec::new() };
        
        let score = engine.calculate_step_reward(Some(&tr), Some(&dr), 500, 1000, true);
        
        assert!(score.success > 9.0);
        assert_eq!(score.quality_penalty, 0.0);
        assert!(score.total > 11.0); // Success (10) + Forensic (2) - slight efficiency penalty
    }

    #[test]
    fn evaluate_trajectory_averages_correctly() {
        let engine = RewardEngine::new(RewardConfig::default());
        let r1 = RewardScore { total: 10.0, ..Default::default() };
        let r2 = RewardScore { total: 0.0, ..Default::default() };
        
        let avg = engine.evaluate_trajectory(&[r1, r2]);
        assert_eq!(avg, 5.0);
    }
}
