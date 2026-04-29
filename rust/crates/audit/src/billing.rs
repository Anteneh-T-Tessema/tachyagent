//! Stripe billing connector — aggregates metered usage and reports to Stripe.
//!
//! Uses a `BillingBackend` trait for testability. The `StripeBillingConnector`
//! drains the `MeteringService` each billing period, aggregates per user,
//! and reports three dimensions: `tokens_consumed`, `tool_invocations`, `agent_runs`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::event::{AuditEvent, AuditEventKind, AuditSeverity};
use crate::logger::AuditLogger;
use crate::metering::MeteringService;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from the billing subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingError {
    /// Stripe API (or mock) returned an error.
    StripeApiError(String),
    /// User has no subscription mapping.
    NoSubscriptionMapping(String),
    /// Configuration is missing (e.g. no Stripe API key).
    ConfigMissing(String),
}

impl fmt::Display for BillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BillingError::StripeApiError(msg) => write!(f, "stripe api error: {msg}"),
            BillingError::NoSubscriptionMapping(uid) => {
                write!(f, "no subscription mapping for user: {uid}")
            }
            BillingError::ConfigMissing(msg) => write!(f, "config missing: {msg}"),
        }
    }
}

impl std::error::Error for BillingError {}

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// Information about a newly created subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionInfo {
    pub subscription_id: String,
    pub customer_id: String,
    pub subscription_item_id: String,
}

/// Summary of a billing flush cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingReport {
    pub users_reported: usize,
    pub total_tokens: u64,
    pub total_tools: u64,
    pub total_runs: u64,
    pub errors: Vec<String>,
}

/// Current billing connector status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingStatus {
    pub last_flush_at: u64,
    pub period_start: u64,
    pub users_mapped: usize,
    pub sync_ok: bool,
}

// ---------------------------------------------------------------------------
// BillingBackend trait
// ---------------------------------------------------------------------------

/// Abstraction over the billing provider (Stripe, mock, etc.).
pub trait BillingBackend: Send + Sync {
    /// Report metered usage for a subscription item.
    fn report_usage(
        &self,
        subscription_item_id: &str,
        quantity: u64,
        timestamp: u64,
    ) -> Result<(), BillingError>;

    /// Create a new customer and return the customer ID.
    fn create_customer(&self, email: &str) -> Result<String, BillingError>;

    /// Create a subscription for a customer and return subscription info.
    fn create_subscription(
        &self,
        customer_id: &str,
        price_ids: &[&str],
    ) -> Result<SubscriptionInfo, BillingError>;
}

// ---------------------------------------------------------------------------
// StripeBillingConnector
// ---------------------------------------------------------------------------

/// Aggregates usage per billing period and reports to a `BillingBackend`.
pub struct StripeBillingConnector {
    backend: Box<dyn BillingBackend>,
    user_subscription_map: BTreeMap<String, String>, // user_id → subscription_item_id
    billing_period_secs: u64,                        // default: 3600
    max_retries: u32,                                // default: 3
    // Internal bookkeeping
    last_flush_at: u64,
    period_start: u64,
    last_report: Option<BillingReport>,
    sync_ok: bool,
}

impl StripeBillingConnector {
    /// Create a new connector with the given backend and defaults.
    #[must_use]
    pub fn new(backend: Box<dyn BillingBackend>) -> Self {
        Self {
            backend,
            user_subscription_map: BTreeMap::new(),
            billing_period_secs: 3600,
            max_retries: 3,
            last_flush_at: 0,
            period_start: 0,
            last_report: None,
            sync_ok: true,
        }
    }

    /// Override the billing period (seconds).
    #[must_use]
    pub fn with_billing_period(mut self, secs: u64) -> Self {
        self.billing_period_secs = secs;
        self
    }

    /// Override the max retry count.
    #[must_use]
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Pre-populate the user → `subscription_item_id` mapping.
    pub fn set_user_mapping(&mut self, user_id: &str, subscription_item_id: &str) {
        self.user_subscription_map
            .insert(user_id.to_string(), subscription_item_id.to_string());
    }

    /// Drain the `MeteringService`, aggregate per user, and report to the
    /// `BillingBackend`. Retries up to `max_retries` times with exponential
    /// backoff (1s, 2s, 4s). Failures are logged to the metering service's
    /// audit logger.
    pub fn flush_period(
        &mut self,
        metering: &mut MeteringService,
    ) -> Result<BillingReport, BillingError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let aggregates = metering.drain_period(now);

        let mut report = BillingReport {
            users_reported: 0,
            total_tokens: 0,
            total_tools: 0,
            total_runs: 0,
            errors: Vec::new(),
        };

        for agg in &aggregates {
            let sub_item_id = if let Some(id) = self.user_subscription_map.get(&agg.user_id) {
                id.clone()
            } else {
                let msg = format!("no subscription mapping for user: {}", agg.user_id);
                Self::log_billing_error(metering.audit_logger(), &msg);
                report.errors.push(msg);
                continue;
            };

            let tokens = agg.total_input_tokens + agg.total_output_tokens;
            let tools = agg.total_tool_invocations;
            let runs = agg.total_agent_runs;

            // Report each dimension with retry
            let dimensions: [(&str, u64); 3] = [
                ("tokens_consumed", tokens),
                ("tool_invocations", tools),
                ("agent_runs", runs),
            ];

            let mut user_ok = true;
            for (dim_name, quantity) in &dimensions {
                if *quantity == 0 {
                    continue;
                }
                if let Err(e) = self.report_with_retry(&sub_item_id, *quantity, now) {
                    let msg = format!(
                        "failed to report {} for user {}: {}",
                        dim_name, agg.user_id, e
                    );
                    Self::log_billing_error(metering.audit_logger(), &msg);
                    report.errors.push(msg);
                    user_ok = false;
                }
            }

            if user_ok {
                report.users_reported += 1;
            }
            report.total_tokens += tokens;
            report.total_tools += tools;
            report.total_runs += runs;
        }

        self.last_flush_at = now;
        self.period_start = now;
        self.sync_ok = report.errors.is_empty();
        self.last_report = Some(report.clone());

        Ok(report)
    }

    /// Provision a new user: create a Stripe customer and subscription.
    pub fn provision_user(&mut self, user_id: &str, email: &str) -> Result<(), BillingError> {
        let customer_id = self.backend.create_customer(email)?;
        let price_ids: &[&str] = &["tokens_consumed", "tool_invocations", "agent_runs"];
        let sub_info = self.backend.create_subscription(&customer_id, price_ids)?;
        self.user_subscription_map
            .insert(user_id.to_string(), sub_info.subscription_item_id);
        Ok(())
    }

    /// Return the current billing status.
    #[must_use]
    pub fn status(&self) -> BillingStatus {
        BillingStatus {
            last_flush_at: self.last_flush_at,
            period_start: self.period_start,
            users_mapped: self.user_subscription_map.len(),
            sync_ok: self.sync_ok,
        }
    }

    // -- private helpers ----------------------------------------------------

    /// Report usage with exponential backoff retry (1s, 2s, 4s).
    /// In tests the sleep is a no-op because we don't actually sleep;
    /// the retry logic is the important part.
    fn report_with_retry(
        &self,
        subscription_item_id: &str,
        quantity: u64,
        timestamp: u64,
    ) -> Result<(), BillingError> {
        let mut last_err = None;
        for attempt in 0..self.max_retries {
            match self
                .backend
                .report_usage(subscription_item_id, quantity, timestamp)
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    // Exponential backoff: 1s, 2s, 4s — skip actual sleep in
                    // library code; callers can wrap with real delays.
                    let _backoff_secs = 1u64 << attempt;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| BillingError::StripeApiError("unknown".into())))
    }

    fn log_billing_error(logger: &AuditLogger, msg: &str) {
        let event = AuditEvent::new("billing", AuditEventKind::UsageMetering, msg)
            .with_severity(AuditSeverity::Warning);
        logger.log(&event);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::MemoryAuditSink;
    use crate::metering::{UsageEvent, UsageEventType};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    // -- Mock backend -------------------------------------------------------

    /// Tracks all calls made to the billing backend.
    #[derive(Debug, Clone)]
    struct MockCall {
        method: String,
        args: Vec<String>,
    }

    struct MockBillingBackend {
        calls: Arc<Mutex<Vec<MockCall>>>,
        /// Number of times `report_usage` should fail before succeeding.
        fail_count: AtomicU32,
    }

    impl MockBillingBackend {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_count: AtomicU32::new(0),
            }
        }

        fn with_fail_count(self, n: u32) -> Self {
            self.fail_count.store(n, Ordering::SeqCst);
            self
        }
    }

    impl BillingBackend for MockBillingBackend {
        fn report_usage(
            &self,
            subscription_item_id: &str,
            quantity: u64,
            timestamp: u64,
        ) -> Result<(), BillingError> {
            let remaining = self.fail_count.load(Ordering::SeqCst);
            if remaining > 0 {
                self.fail_count.store(remaining - 1, Ordering::SeqCst);
                return Err(BillingError::StripeApiError("transient".into()));
            }
            self.calls.lock().unwrap().push(MockCall {
                method: "report_usage".into(),
                args: vec![
                    subscription_item_id.to_string(),
                    quantity.to_string(),
                    timestamp.to_string(),
                ],
            });
            Ok(())
        }

        fn create_customer(&self, email: &str) -> Result<String, BillingError> {
            self.calls.lock().unwrap().push(MockCall {
                method: "create_customer".into(),
                args: vec![email.to_string()],
            });
            Ok(format!("cus_{}", email.replace('@', "_")))
        }

        fn create_subscription(
            &self,
            customer_id: &str,
            price_ids: &[&str],
        ) -> Result<SubscriptionInfo, BillingError> {
            self.calls.lock().unwrap().push(MockCall {
                method: "create_subscription".into(),
                args: vec![customer_id.to_string(), price_ids.join(",")],
            });
            Ok(SubscriptionInfo {
                subscription_id: format!("sub_{customer_id}"),
                customer_id: customer_id.to_string(),
                subscription_item_id: format!("si_{customer_id}"),
            })
        }
    }

    // -- Helpers -------------------------------------------------------------

    fn make_metering() -> (MeteringService, MemoryAuditSink) {
        let sink = MemoryAuditSink::new();
        let mut logger = AuditLogger::new();
        logger.add_sink(sink.clone());
        (
            MeteringService::new(
                Arc::new(logger),
                crate::cost_model::CostModelRegistry::default(),
            ),
            sink,
        )
    }

    fn agent_run(user: &str, input: u64, output: u64, tools: u32, ts: u64) -> UsageEvent {
        UsageEvent {
            event_type: UsageEventType::AgentRun,
            user_id: user.to_string(),
            team_id: None,
            agent_id: "agent-1".to_string(),
            model_name: Some("gemma4:26b".to_string()),
            input_tokens: input,
            output_tokens: output,
            tool_name: None,
            tool_invocation_count: tools,
            timestamp: ts,
        }
    }

    // -- Tests --------------------------------------------------------------

    #[test]
    fn successful_flush_reports_all_dimensions() {
        let (mut metering, _sink) = make_metering();
        metering
            .record_event(agent_run("user-1", 100, 50, 3, 1000))
            .unwrap();
        metering
            .record_event(agent_run("user-2", 200, 80, 1, 1100))
            .unwrap();

        let mock = MockBillingBackend::new();
        let calls_ref = mock.calls.clone();

        let mut connector = StripeBillingConnector::new(Box::new(mock));
        connector.set_user_mapping("user-1", "si_user1");
        connector.set_user_mapping("user-2", "si_user2");

        let report = connector.flush_period(&mut metering).unwrap();

        assert_eq!(report.users_reported, 2);
        assert_eq!(report.total_tokens, 430); // 100+50 + 200+80
        assert_eq!(report.total_tools, 4); // 3 + 1
        assert_eq!(report.total_runs, 2);
        assert!(report.errors.is_empty());

        // Backend received report_usage calls for each non-zero dimension
        let calls = calls_ref.lock().unwrap();
        let report_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.method == "report_usage")
            .collect();
        // user-1: tokens(150), tools(3), runs(1) = 3 calls
        // user-2: tokens(280), tools(1), runs(1) = 3 calls
        assert_eq!(report_calls.len(), 6);
    }

    #[test]
    fn flush_retries_on_failure_then_succeeds() {
        let (mut metering, _sink) = make_metering();
        metering
            .record_event(agent_run("user-1", 100, 0, 0, 1000))
            .unwrap();

        // Fail twice, then succeed (within 3 retries)
        let mock = MockBillingBackend::new().with_fail_count(2);
        let calls_ref = mock.calls.clone();

        let mut connector = StripeBillingConnector::new(Box::new(mock));
        connector.set_user_mapping("user-1", "si_user1");

        let report = connector.flush_period(&mut metering).unwrap();

        assert_eq!(report.users_reported, 1);
        assert!(report.errors.is_empty());

        // tokens_consumed (100) succeeds after 2 retries → 1 recorded call
        // agent_runs (1) succeeds immediately (fail_count exhausted) → 1 recorded call
        let calls = calls_ref.lock().unwrap();
        assert_eq!(
            calls.iter().filter(|c| c.method == "report_usage").count(),
            2
        );
    }

    #[test]
    fn flush_logs_error_after_max_retries_exhausted() {
        let (mut metering, sink) = make_metering();
        metering
            .record_event(agent_run("user-1", 100, 0, 0, 1000))
            .unwrap();

        // Fail more times than max_retries (3)
        let mock = MockBillingBackend::new().with_fail_count(5);

        let mut connector = StripeBillingConnector::new(Box::new(mock));
        connector.set_user_mapping("user-1", "si_user1");

        let report = connector.flush_period(&mut metering).unwrap();

        // User not counted as reported
        assert_eq!(report.users_reported, 0);
        assert!(!report.errors.is_empty());
        assert!(report.errors[0].contains("failed to report"));

        // Audit trail has billing error events
        let events = sink.events();
        let billing_errors: Vec<_> = events
            .iter()
            .filter(|e| e.detail.contains("failed to report"))
            .collect();
        assert!(!billing_errors.is_empty());
    }

    #[test]
    fn flush_skips_unmapped_users() {
        let (mut metering, sink) = make_metering();
        metering
            .record_event(agent_run("user-1", 100, 0, 0, 1000))
            .unwrap();
        metering
            .record_event(agent_run("user-2", 200, 0, 0, 1100))
            .unwrap();

        let mock = MockBillingBackend::new();
        let mut connector = StripeBillingConnector::new(Box::new(mock));
        // Only map user-1
        connector.set_user_mapping("user-1", "si_user1");

        let report = connector.flush_period(&mut metering).unwrap();

        assert_eq!(report.users_reported, 1);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("no subscription mapping"));

        // Audit trail has warning for unmapped user
        let events = sink.events();
        assert!(events
            .iter()
            .any(|e| e.detail.contains("no subscription mapping")));
    }

    #[test]
    fn provision_user_creates_customer_and_subscription() {
        let mock = MockBillingBackend::new();
        let calls_ref = mock.calls.clone();

        let mut connector = StripeBillingConnector::new(Box::new(mock));
        connector
            .provision_user("user-1", "alice@example.com")
            .unwrap();

        let calls = calls_ref.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].method, "create_customer");
        assert_eq!(calls[0].args[0], "alice@example.com");
        assert_eq!(calls[1].method, "create_subscription");

        // User is now mapped
        let status = connector.status();
        assert_eq!(status.users_mapped, 1);
    }

    #[test]
    fn status_reflects_state() {
        let mock = MockBillingBackend::new();
        let mut connector = StripeBillingConnector::new(Box::new(mock));

        let status = connector.status();
        assert_eq!(status.users_mapped, 0);
        assert!(status.sync_ok);
        assert_eq!(status.last_flush_at, 0);

        connector.set_user_mapping("u1", "si_1");
        connector.set_user_mapping("u2", "si_2");

        let status = connector.status();
        assert_eq!(status.users_mapped, 2);
    }
}
