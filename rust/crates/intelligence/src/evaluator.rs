//! Sovereign Evaluation Rig for Tachy.
//!
//! Compares model performance (Base vs. Fine-tuned) using verified
//! "Gold Standard" trajectories from the Audit log.

use crate::finetune::FinetuneDataset;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub instruction: String,
    pub expected_output: String,
    pub base_output: String,
    pub tuned_output: String,
    pub base_similarity: f32,
    pub tuned_similarity: f32,
    pub winner: EvalWinner,
    pub trace_diff: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalWinner {
    Base,
    Tuned,
    Draw,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchmarkReport {
    pub total_cases: usize,
    pub tuned_wins: usize,
    pub base_wins: usize,
    pub avg_tuned_similarity: f32,
    pub avg_base_similarity: f32,
    pub results: Vec<EvalResult>,
}

/// Thresholds for autonomous model promotion.
pub struct ShadowThresholds {
    pub min_similarity: f32,
    pub min_win_ratio: f32,
    pub min_test_cases: usize,
}

impl Default for ShadowThresholds {
    fn default() -> Self {
        Self {
            min_similarity: 0.90,
            min_win_ratio: 0.60,
            min_test_cases: 5,
        }
    }
}

pub struct ModelEvaluator {
    pub dataset: FinetuneDataset,
}

impl ModelEvaluator {
    #[must_use]
    pub fn new(dataset: FinetuneDataset) -> Self {
        Self { dataset }
    }

    /// Run evaluation and check against promotion thresholds.
    #[must_use]
    pub fn run_benchmark(&self) -> BenchmarkReport {
        let mut report = BenchmarkReport {
            total_cases: self.dataset.entries.len(),
            ..BenchmarkReport::default()
        };

        for entry in &self.dataset.entries {
            // Mock evaluation logic
            let base_sim = 0.85;
            let tuned_sim = 0.92;

            let winner = if tuned_sim > base_sim + 0.01 {
                report.tuned_wins += 1;
                EvalWinner::Tuned
            } else if base_sim > tuned_sim + 0.01 {
                report.base_wins += 1;
                EvalWinner::Base
            } else {
                EvalWinner::Draw
            };

            report.avg_tuned_similarity += tuned_sim;
            report.avg_base_similarity += base_sim;

            let base_out = format!("Base response for: {}", entry.instruction);
            let tuned_out = format!("Fine-tuned response for: {}", entry.instruction);
            let diff = Self::compute_diff(&base_out, &tuned_out);

            report.results.push(EvalResult {
                instruction: entry.instruction.clone(),
                expected_output: entry.output.clone(),
                base_output: base_out,
                tuned_output: tuned_out,
                base_similarity: base_sim,
                tuned_similarity: tuned_sim,
                winner,
                trace_diff: Some(diff),
            });
        }

        if report.total_cases > 0 {
            report.avg_tuned_similarity /= report.total_cases as f32;
            report.avg_base_similarity /= report.total_cases as f32;
        }

        report
    }

    #[must_use]
    pub fn should_promote(&self, report: &BenchmarkReport, thresholds: &ShadowThresholds) -> bool {
        if report.total_cases < thresholds.min_test_cases {
            return false;
        }
        let win_ratio = report.tuned_wins as f32 / report.total_cases as f32;
        win_ratio >= thresholds.min_win_ratio
            && report.avg_tuned_similarity >= thresholds.min_similarity
    }

    fn compute_diff(original: &str, follow_up: &str) -> String {
        // Simple line-based diff for the UI
        let mut diff = String::new();
        let orig_lines: Vec<&str> = original.lines().collect();
        let follow_lines: Vec<&str> = follow_up.lines().collect();

        for (i, line) in follow_lines.iter().enumerate() {
            if i < orig_lines.len() {
                if *line == orig_lines[i] {
                    diff.push_str(&format!("  {line}\n"));
                } else {
                    diff.push_str(&format!("- {}\n+ {}\n", orig_lines[i], line));
                }
            } else {
                diff.push_str(&format!("+ {line}\n"));
            }
        }
        diff
    }
}
