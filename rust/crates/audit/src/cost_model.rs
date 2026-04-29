//! Dynamic Cost Model for Tachy.
//!
//! Loads pricing data from `.tachy/config/pricing.json` and provides
//! USD cost calculations for different models and tiers.

use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub name: String,
    /// Cost per 1,000 input tokens (USD).
    pub input_cost_per_1k: f64,
    /// Cost per 1,000 output tokens (USD).
    pub output_cost_per_1k: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostModelRegistry {
    pub models: HashMap<String, ModelPricing>,
    /// Default cost if model is not in registry.
    pub default_input_cost_per_1k: f64,
    pub default_output_cost_per_1k: f64,
}

impl CostModelRegistry {
    /// Load pricing from disk.
    pub fn load(tachy_dir: &Path) -> Self {
        let path = tachy_dir.join("config").join("pricing.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(reg) = serde_json::from_str::<CostModelRegistry>(&content) {
                return reg;
            }
        }
        
        // Return defaults (SaaSw $0.002/1k tokens standard)
        let mut reg = CostModelRegistry {
            models: HashMap::new(),
            default_input_cost_per_1k: 0.002,
            default_output_cost_per_1k: 0.002,
        };
        
        // Add common defaults
        reg.models.insert("gemma4:26b".to_string(), ModelPricing {
            name: "gemma4:26b".to_string(),
            input_cost_per_1k: 0.002,
            output_cost_per_1k: 0.002,
        });
        reg.models.insert("gpt-4o".to_string(), ModelPricing {
            name: "gpt-4o".to_string(),
            input_cost_per_1k: 0.005,
            output_cost_per_1k: 0.015,
        });
        
        reg
    }

    /// Calculate cost in USD.
    pub fn calculate_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
        let pricing = self.models.get(model);
        let input_rate = pricing.map(|p| p.input_cost_per_1k).unwrap_or(self.default_input_cost_per_1k);
        let output_rate = pricing.map(|p| p.output_cost_per_1k).unwrap_or(self.default_output_cost_per_1k);
        
        let input_cost = (input_tokens as f64 / 1000.0) * input_rate;
        let output_cost = (output_tokens as f64 / 1000.0) * output_rate;
        
        input_cost + output_cost
    }
}
