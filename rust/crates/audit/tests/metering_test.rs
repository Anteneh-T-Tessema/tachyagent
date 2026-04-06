//! Metering property tests.
//!
//! Feature: product-hardening-v3
//! Properties 1–3: MeteringService correctness.
//! Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.6

use proptest::prelude::*;
use audit::{AuditLogger, MeteringService, UsageEvent, UsageEventType, MeteringError};

fn make_agent_run_event(user_id: &str, input: u64, output: u64, tools: u32, ts: u64) -> UsageEvent {
    UsageEvent {
        event_type: UsageEventType::AgentRun,
        user_id: user_id.to_string(),
        team_id: None,
        agent_id: "agent-test".to_string(),
        model_name: Some("gemma4:26b".to_string()),
        input_tokens: input,
        output_tokens: output,
        tool_name: None,
        tool_invocation_count: tools,
        timestamp: ts,
    }
}

fn make_tool_event(user_id: &str, tool: &str, ts: u64) -> UsageEvent {
    UsageEvent {
        event_type: UsageEventType::ToolInvocation,
        user_id: user_id.to_string(),
        team_id: None,
        agent_id: "agent-test".to_string(),
        model_name: None,
        input_tokens: 0,
        output_tokens: 0,
        tool_name: Some(tool.to_string()),
        tool_invocation_count: 1,
        timestamp: ts,
    }
}

// ---------------------------------------------------------------------------
// Property 1: Usage event recording preserves all fields and produces audit entry
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 1: Recording a valid usage event updates counters correctly
    /// and does not return an error.
    ///
    /// Feature: product-hardening-v3, Property 1: Usage event recording preserves all fields
    #[test]
    fn prop_record_event_updates_counters(
        input_tokens in 1u64..10_000u64,
        output_tokens in 1u64..10_000u64,
        tool_count in 0u32..20u32,
        ts in 1_000_000u64..9_999_999u64,
    ) {
        let mut svc = MeteringService::new(AuditLogger::new());
        let event = make_agent_run_event("user-1", input_tokens, output_tokens, tool_count, ts);

        prop_assert!(svc.record_event(event).is_ok(), "valid event should not fail");

        let agg = svc.get_usage("user-1", 0, u64::MAX);
        prop_assert!(agg.is_some(), "usage should be aggregated");
        let agg = agg.unwrap();
        prop_assert_eq!(agg.total_input_tokens, input_tokens);
        prop_assert_eq!(agg.total_output_tokens, output_tokens);
        prop_assert_eq!(agg.total_agent_runs, 1);
        prop_assert_eq!(agg.total_tool_invocations, u64::from(tool_count));
    }

    /// Property 1b: Multiple events for the same user accumulate linearly.
    #[test]
    fn prop_multiple_events_accumulate(
        n in 2usize..10usize,
        tokens_per_event in 1u64..1_000u64,
    ) {
        // Feature: product-hardening-v3, Property 1: Usage event recording preserves all fields
        let mut svc = MeteringService::new(AuditLogger::new());

        for i in 0..n {
            let event = make_agent_run_event("user-acc", tokens_per_event, tokens_per_event, 0, i as u64 + 1);
            svc.record_event(event).unwrap();
        }

        let agg = svc.get_usage("user-acc", 0, u64::MAX).unwrap();
        prop_assert_eq!(agg.total_input_tokens, tokens_per_event * n as u64);
        prop_assert_eq!(agg.total_output_tokens, tokens_per_event * n as u64);
        prop_assert_eq!(agg.total_agent_runs, n as u64);
    }
}

// ---------------------------------------------------------------------------
// Property 2: Counter consistency
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 2: For N events across M distinct users, the total tokens summed
    /// across all users equals the sum of all individual events.
    ///
    /// Feature: product-hardening-v3, Property 2: Counter consistency
    #[test]
    fn prop_counter_consistency_across_users(
        user_count in 2usize..5usize,
        events_per_user in 1usize..6usize,
        tokens in 10u64..500u64,
    ) {
        let mut svc = MeteringService::new(AuditLogger::new());
        let mut expected_total = 0u64;

        for u in 0..user_count {
            let uid = format!("user-{u}");
            for i in 0..events_per_user {
                let evt = make_agent_run_event(&uid, tokens, tokens, 0, (u * 100 + i) as u64 + 1);
                svc.record_event(evt).unwrap();
                expected_total += tokens;
            }
        }

        // Sum totals across all users
        let actual_total: u64 = (0..user_count).map(|u| {
            let uid = format!("user-{u}");
            svc.get_usage(&uid, 0, u64::MAX)
                .map(|a| a.total_input_tokens)
                .unwrap_or(0)
        }).sum();

        prop_assert_eq!(actual_total, expected_total);
    }

    /// Property 2b: Tool invocations accumulate independently of agent runs.
    #[test]
    fn prop_tool_and_run_counters_independent(
        run_count in 1usize..5usize,
        tool_count in 1usize..5usize,
    ) {
        // Feature: product-hardening-v3, Property 2: Counter consistency
        let mut svc = MeteringService::new(AuditLogger::new());
        let uid = "user-indep";

        for i in 0..run_count {
            svc.record_event(make_agent_run_event(uid, 100, 100, 0, i as u64 + 1)).unwrap();
        }
        for i in 0..tool_count {
            svc.record_event(make_tool_event(uid, "read_file", (run_count + i) as u64 + 1)).unwrap();
        }

        let agg = svc.get_usage(uid, 0, u64::MAX).unwrap();
        prop_assert_eq!(agg.total_agent_runs, run_count as u64);
        prop_assert_eq!(agg.total_tool_invocations, tool_count as u64);
    }
}

// ---------------------------------------------------------------------------
// Property 3: Invalid usage events are rejected
// ---------------------------------------------------------------------------

#[test]
fn empty_user_id_is_rejected() {
    // Feature: product-hardening-v3, Property 3: Invalid usage events are rejected
    let mut svc = MeteringService::new(AuditLogger::new());
    let event = make_agent_run_event("", 100, 100, 0, 1);
    assert!(
        matches!(svc.record_event(event), Err(MeteringError::InvalidEvent(_))),
        "empty user_id must be rejected"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 3: Events with empty user_id are always rejected.
    ///
    /// Feature: product-hardening-v3, Property 3: Invalid usage events are rejected
    #[test]
    fn prop_empty_user_id_rejected(
        input in 0u64..10_000u64,
        output in 0u64..10_000u64,
    ) {
        let mut svc = MeteringService::new(AuditLogger::new());
        let event = make_agent_run_event("", input, output, 0, 1);
        prop_assert!(svc.record_event(event).is_err(), "empty user_id must fail");
    }

    /// Property 3b: Valid user IDs are never rejected.
    #[test]
    fn prop_valid_events_always_accepted(
        user_id in "[a-z][a-z0-9\\-]{2,16}",
        input in 0u64..100_000u64,
        output in 0u64..100_000u64,
    ) {
        // Feature: product-hardening-v3, Property 3: Invalid usage events are rejected
        let mut svc = MeteringService::new(AuditLogger::new());
        let event = make_agent_run_event(&user_id, input, output, 0, 1);
        prop_assert!(svc.record_event(event).is_ok(), "valid event must be accepted");
    }
}
