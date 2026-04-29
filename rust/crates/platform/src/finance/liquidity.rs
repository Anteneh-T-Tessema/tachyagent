//! Sovereign Liquidity Engine — autonomous financial orchestration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolRate {
    pub protocol: String,
    pub asset: String,
    pub apy: f32,
    pub liquidity_depth: f64,
    pub risk_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrageOpportunity {
    pub source_protocol: String,
    pub target_protocol: String,
    pub asset: String,
    pub spread: f32,
    pub expected_yield_boost: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestmentPolicy {
    pub max_exposure_per_protocol: f64,
    pub min_apy_threshold: f32,
    pub max_slippage: f32,
    pub restricted_protocols: Vec<String>,
}

pub struct LiquidityMonitor {
    rates: HashMap<String, ProtocolRate>,
}

impl LiquidityMonitor {
    pub fn new() -> Self {
        let mut rates = HashMap::new();
        // Initial protocol stubs
        rates.insert("Aave-v3".to_string(), ProtocolRate {
            protocol: "Aave-v3".to_string(),
            asset: "USDC".to_string(),
            apy: 0.042,
            liquidity_depth: 1_200_000_000.0,
            risk_score: 0.95,
        });
        rates.insert("Compound-v3".to_string(), ProtocolRate {
            protocol: "Compound-v3".to_string(),
            asset: "USDC".to_string(),
            apy: 0.038,
            liquidity_depth: 850_000_000.0,
            risk_score: 0.92,
        });
        
        Self { rates }
    }

    pub fn get_rates(&self) -> Vec<ProtocolRate> {
        self.rates.values().cloned().collect()
    }
}

pub struct StrategyEngine {
    policy: InvestmentPolicy,
}

impl StrategyEngine {
    pub fn new(policy: InvestmentPolicy) -> Self {
        Self { policy }
    }

    pub fn find_opportunities(&self, monitor: &LiquidityMonitor) -> Vec<ArbitrageOpportunity> {
        let rates = monitor.get_rates();
        let mut opportunities = Vec::new();

        for r1 in &rates {
            for r2 in &rates {
                if r1.protocol == r2.protocol || r1.asset != r2.asset {
                    continue;
                }

                let spread = r1.apy - r2.apy;
                if spread > 0.005 { // 0.5% threshold
                    opportunities.push(ArbitrageOpportunity {
                        source_protocol: r2.protocol.clone(),
                        target_protocol: r1.protocol.clone(),
                        asset: r1.asset.clone(),
                        spread,
                        expected_yield_boost: spread * 100.0,
                    });
                }
            }
        }

        opportunities
    }
}

impl Default for InvestmentPolicy {
    fn default() -> Self {
        Self {
            max_exposure_per_protocol: 100_000.0, // $100k
            min_apy_threshold: 0.02,             // 2%
            max_slippage: 0.001,                 // 0.1%
            restricted_protocols: vec!["TornadoCash".to_string()],
        }
    }
}
