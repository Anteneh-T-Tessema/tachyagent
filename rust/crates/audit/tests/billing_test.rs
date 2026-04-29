//! Billing property tests.
//!
//! Feature: product-hardening-v3
//! Property 4: Billing aggregation reports correct totals per user.
//! Validates: Requirements 2.1, 2.4

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};

use audit::cost_model::CostModelRegistry;
use audit::{
    billing::{BillingBackend, SubscriptionInfo},
    AuditLogger, BillingError, MeteringService, StripeBillingConnector, UsageEvent, UsageEventType,
};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Mock billing backend
// ---------------------------------------------------------------------------

struct MockBackend {
    calls: Arc<Mutex<Vec<u64>>>,
    fail_count: AtomicU32,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_count: AtomicU32::new(0),
        }
    }
}

impl BillingBackend for MockBackend {
    fn report_usage(&self, _sub_id: &str, qty: u64, _ts: u64) -> Result<(), BillingError> {
        let rem = self.fail_count.load(Ordering::SeqCst);
        if rem > 0 {
            self.fail_count.store(rem - 1, Ordering::SeqCst);
            return Err(BillingError::StripeApiError("transient".into()));
        }
        self.calls.lock().unwrap().push(qty);
        Ok(())
    }

    fn create_customer(&self, email: &str) -> Result<String, BillingError> {
        Ok(format!("cus_{}", email.replace('@', "_")))
    }

    fn create_subscription(
        &self,
        customer_id: &str,
        _price_ids: &[&str],
    ) -> Result<SubscriptionInfo, BillingError> {
        Ok(SubscriptionInfo {
            subscription_id: format!("sub_{customer_id}"),
            customer_id: customer_id.to_string(),
            subscription_item_id: format!("si_{customer_id}"),
        })
    }
}

fn make_event(user_id: &str, input: u64, output: u64, tools: u32, ts: u64) -> UsageEvent {
    UsageEvent {
        event_type: UsageEventType::AgentRun,
        user_id: user_id.to_string(),
        team_id: None,
        agent_id: "agent-1".to_string(),
        model_name: None,
        input_tokens: input,
        output_tokens: output,
        tool_name: None,
        tool_invocation_count: tools,
        timestamp: ts,
    }
}

// ---------------------------------------------------------------------------
// Property 4: Billing aggregation reports correct totals per user
// ---------------------------------------------------------------------------

/// Verify `flush_period` calls `report_usage` with the correct combined token total.
#[test]
fn flush_period_reports_correct_totals() {
    // Feature: product-hardening-v3, Property 4: Billing aggregation reports correct totals
    let mut metering =
        MeteringService::new(Arc::new(AuditLogger::new()), CostModelRegistry::default());

    // Record 3 events: (100+50) + (200+100) + (300+150) = 150 + 300 + 450 = 900
    metering
        .record_event(make_event("user-a", 100, 50, 0, 1))
        .unwrap();
    metering
        .record_event(make_event("user-a", 200, 100, 2, 2))
        .unwrap();
    metering
        .record_event(make_event("user-a", 300, 150, 1, 3))
        .unwrap();

    let expected_tokens = 900u64; // sum of (input + output) across all events

    let mock = MockBackend::new();
    let mut connector = StripeBillingConnector::new(Box::new(mock));
    connector.set_user_mapping("user-a", "si_abc123");

    let report = connector.flush_period(&mut metering).unwrap();

    assert_eq!(
        report.total_tokens, expected_tokens,
        "reported total_tokens should equal sum of input+output"
    );
    assert_eq!(
        report.users_reported, 1,
        "should report for exactly one user"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 4: For any N events each contributing (input + output) tokens,
    /// flush_period reports exactly the correct combined total.
    ///
    /// Feature: product-hardening-v3, Property 4: Billing aggregation reports correct totals
    #[test]
    fn prop_flush_reports_exact_token_total(
        events in prop::collection::vec(
            (1u64..500u64, 1u64..500u64, 0u32..5u32),
            1..8usize,
        )
    ) {
        let mut metering = MeteringService::new(Arc::new(AuditLogger::new()), CostModelRegistry::default());

        let mut expected_tokens = 0u64;
        for (i, (input, output, tools)) in events.iter().enumerate() {
            metering.record_event(make_event("user-prop", *input, *output, *tools, i as u64 + 1)).unwrap();
            expected_tokens += input + output;
        }

        let mock = MockBackend::new();
        let mut connector = StripeBillingConnector::new(Box::new(mock));
        connector.set_user_mapping("user-prop", "si_test");

        let report = connector.flush_period(&mut metering).unwrap();

        prop_assert_eq!(report.total_tokens, expected_tokens,
            "reported total_tokens must match sum of (input+output) across all events");
    }

    /// Property 4b: flush_period with no mapped users reports zero usage.
    #[test]
    fn prop_flush_with_no_mapped_users_is_zero(
        events in prop::collection::vec(
            (1u64..200u64, 1u64..200u64, 0u32..3u32),
            1..5usize,
        )
    ) {
        // Feature: product-hardening-v3, Property 4: Billing aggregation reports correct totals
        let mut metering = MeteringService::new(Arc::new(AuditLogger::new()), CostModelRegistry::default());

        for (i, (input, output, tools)) in events.iter().enumerate() {
            metering.record_event(make_event("unmapped-user", *input, *output, *tools, i as u64 + 1)).unwrap();
        }

        let mock = MockBackend::new();
        // No user mapping set — no one is billed
        let mut connector = StripeBillingConnector::new(Box::new(mock));
        let report = connector.flush_period(&mut metering).unwrap();

        prop_assert_eq!(report.users_reported, 0, "no mapped users means no billing");
        prop_assert_eq!(report.total_tokens, 0, "no mapped users means zero tokens reported");
    }
}
