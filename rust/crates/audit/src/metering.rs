//! Usage metering — tracks token consumption, tool invocations, and agent runs.
//!
//! Records `UsageEvent`s, maintains in-memory counters per user/team,
//! and persists every event to the audit trail with kind `"usage_metering"`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::event::{AuditEvent, AuditEventKind};
use crate::logger::AuditLogger;

/// The type of usage event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageEventType {
    AgentRun,
    ToolInvocation,
}

/// A single usage event to be recorded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub event_type: UsageEventType,
    pub user_id: String,
    pub team_id: Option<String>,
    pub agent_id: String,
    pub model_name: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_name: Option<String>,
    pub tool_invocation_count: u32,
    pub timestamp: u64,
}

/// Aggregated usage counters for a user within a period.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageAggregate {
    pub user_id: String,
    pub team_id: Option<String>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tool_invocations: u64,
    pub total_agent_runs: u64,
    pub period_start: u64,
    pub period_end: u64,
}

/// Errors from the metering service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeteringError {
    /// Event has empty user_id or other invalid fields.
    InvalidEvent(String),
}

impl fmt::Display for MeteringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeteringError::InvalidEvent(msg) => write!(f, "invalid event: {msg}"),
        }
    }
}

impl std::error::Error for MeteringError {}

/// Tracks usage events and maintains in-memory counters keyed by user_id.
pub struct MeteringService {
    counters: BTreeMap<String, UsageAggregate>,
    audit_logger: AuditLogger,
}

impl MeteringService {
    /// Create a new metering service backed by the given audit logger.
    #[must_use]
    pub fn new(audit_logger: AuditLogger) -> Self {
        Self {
            counters: BTreeMap::new(),
            audit_logger,
        }
    }

    /// Record a usage event. Validates the event, persists to audit trail,
    /// and increments in-memory counters.
    pub fn record_event(&mut self, event: UsageEvent) -> Result<(), MeteringError> {
        // Validate
        if event.user_id.is_empty() {
            return Err(MeteringError::InvalidEvent("user_id must not be empty".into()));
        }

        // Build audit event
        let detail = match event.event_type {
            UsageEventType::AgentRun => format!(
                "agent_run: model={}, input_tokens={}, output_tokens={}, tools={}",
                event.model_name.as_deref().unwrap_or("unknown"),
                event.input_tokens,
                event.output_tokens,
                event.tool_invocation_count,
            ),
            UsageEventType::ToolInvocation => format!(
                "tool_invocation: tool={}, agent={}",
                event.tool_name.as_deref().unwrap_or("unknown"),
                event.agent_id,
            ),
        };

        let audit_event = AuditEvent::new("metering", AuditEventKind::UsageMetering, detail)
            .with_user(&event.user_id)
            .with_agent(&event.agent_id);

        let audit_event = if let Some(ref model) = event.model_name {
            audit_event.with_model(model)
        } else {
            audit_event
        };

        let audit_event = if let Some(ref tool) = event.tool_name {
            audit_event.with_tool(tool)
        } else {
            audit_event
        };

        self.audit_logger.log(&audit_event);

        // Update counters
        let agg = self.counters.entry(event.user_id.clone()).or_insert_with(|| {
            UsageAggregate {
                user_id: event.user_id.clone(),
                team_id: event.team_id.clone(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_tool_invocations: 0,
                total_agent_runs: 0,
                period_start: event.timestamp,
                period_end: event.timestamp,
            }
        });

        agg.total_input_tokens += event.input_tokens;
        agg.total_output_tokens += event.output_tokens;

        match event.event_type {
            UsageEventType::AgentRun => {
                agg.total_agent_runs += 1;
                agg.total_tool_invocations += event.tool_invocation_count as u64;
            }
            UsageEventType::ToolInvocation => {
                agg.total_tool_invocations += event.tool_invocation_count as u64;
            }
        }

        // Expand the period window
        if event.timestamp < agg.period_start {
            agg.period_start = event.timestamp;
        }
        if event.timestamp > agg.period_end {
            agg.period_end = event.timestamp;
        }

        // Update team_id if not set yet
        if agg.team_id.is_none() {
            agg.team_id = event.team_id;
        }

        Ok(())
    }

    /// Get usage for a specific user within a time range.
    /// Returns the aggregate if the user exists and their period overlaps [from, to].
    #[must_use]
    pub fn get_usage(&self, user_id: &str, from: u64, to: u64) -> Option<UsageAggregate> {
        self.counters.get(user_id).and_then(|agg| {
            if agg.period_end >= from && agg.period_start <= to {
                Some(agg.clone())
            } else {
                None
            }
        })
    }

    /// Get aggregated usage for all users in a team within a time range.
    #[must_use]
    pub fn get_team_usage(&self, team_id: &str, from: u64, to: u64) -> Option<UsageAggregate> {
        let mut result: Option<UsageAggregate> = None;

        for agg in self.counters.values() {
            if agg.team_id.as_deref() == Some(team_id)
                && agg.period_end >= from
                && agg.period_start <= to
            {
                match result.as_mut() {
                    None => {
                        result = Some(UsageAggregate {
                            user_id: String::new(), // team-level aggregate
                            team_id: Some(team_id.to_string()),
                            total_input_tokens: agg.total_input_tokens,
                            total_output_tokens: agg.total_output_tokens,
                            total_tool_invocations: agg.total_tool_invocations,
                            total_agent_runs: agg.total_agent_runs,
                            period_start: agg.period_start,
                            period_end: agg.period_end,
                        });
                    }
                    Some(ref mut r) => {
                        r.total_input_tokens += agg.total_input_tokens;
                        r.total_output_tokens += agg.total_output_tokens;
                        r.total_tool_invocations += agg.total_tool_invocations;
                        r.total_agent_runs += agg.total_agent_runs;
                        if agg.period_start < r.period_start {
                            r.period_start = agg.period_start;
                        }
                        if agg.period_end > r.period_end {
                            r.period_end = agg.period_end;
                        }
                    }
                }
            }
        }

        result
    }

    /// Drain all counters, setting period_end on each aggregate, and return them.
    /// Resets the in-memory counters.
    pub fn drain_period(&mut self, period_end: u64) -> Vec<UsageAggregate> {
        let old = std::mem::take(&mut self.counters);
        let mut drained: Vec<UsageAggregate> = old
            .into_values()
            .map(|mut agg| {
                agg.period_end = period_end;
                agg
            })
            .collect();
        drained.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        drained
    }

    /// Access the underlying audit logger.
    #[must_use]
    pub fn audit_logger(&self) -> &AuditLogger {
        &self.audit_logger
    }

    /// Access the counters (for testing / billing integration).
    #[must_use]
    pub fn counters(&self) -> &BTreeMap<String, UsageAggregate> {
        &self.counters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::MemoryAuditSink;

    fn make_service() -> (MeteringService, MemoryAuditSink) {
        let sink = MemoryAuditSink::new();
        let mut logger = AuditLogger::new();
        logger.add_sink(sink.clone());
        (MeteringService::new(logger), sink)
    }

    fn agent_run_event(user_id: &str, input: u64, output: u64, tools: u32, ts: u64) -> UsageEvent {
        UsageEvent {
            event_type: UsageEventType::AgentRun,
            user_id: user_id.to_string(),
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

    fn tool_event(user_id: &str, tool: &str, ts: u64) -> UsageEvent {
        UsageEvent {
            event_type: UsageEventType::ToolInvocation,
            user_id: user_id.to_string(),
            team_id: None,
            agent_id: "agent-1".to_string(),
            model_name: None,
            input_tokens: 0,
            output_tokens: 0,
            tool_name: Some(tool.to_string()),
            tool_invocation_count: 1,
            timestamp: ts,
        }
    }

    #[test]
    fn record_valid_agent_run() {
        let (mut svc, sink) = make_service();
        let event = agent_run_event("user-1", 100, 50, 3, 1000);
        svc.record_event(event).unwrap();

        // Counter updated
        let agg = svc.get_usage("user-1", 0, 2000).unwrap();
        assert_eq!(agg.total_input_tokens, 100);
        assert_eq!(agg.total_output_tokens, 50);
        assert_eq!(agg.total_agent_runs, 1);
        assert_eq!(agg.total_tool_invocations, 3);

        // Audit event emitted
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AuditEventKind::UsageMetering);
        assert!(events[0].detail.contains("agent_run"));
    }

    #[test]
    fn record_valid_tool_invocation() {
        let (mut svc, sink) = make_service();
        let event = tool_event("user-1", "bash", 1000);
        svc.record_event(event).unwrap();

        let agg = svc.get_usage("user-1", 0, 2000).unwrap();
        assert_eq!(agg.total_tool_invocations, 1);
        assert_eq!(agg.total_agent_runs, 0);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(events[0].detail.contains("tool_invocation"));
    }

    #[test]
    fn reject_empty_user_id() {
        let (mut svc, sink) = make_service();
        let event = agent_run_event("", 100, 50, 0, 1000);
        let result = svc.record_event(event);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MeteringError::InvalidEvent(_)));

        // No counters updated
        assert!(svc.counters().is_empty());
        // No audit event emitted
        assert!(sink.events().is_empty());
    }

    #[test]
    fn counter_arithmetic_multiple_events() {
        let (mut svc, _) = make_service();

        svc.record_event(agent_run_event("user-1", 100, 50, 2, 1000)).unwrap();
        svc.record_event(agent_run_event("user-1", 200, 80, 1, 1100)).unwrap();
        svc.record_event(tool_event("user-1", "bash", 1050)).unwrap();

        let agg = svc.get_usage("user-1", 0, 2000).unwrap();
        assert_eq!(agg.total_input_tokens, 300);
        assert_eq!(agg.total_output_tokens, 130);
        assert_eq!(agg.total_agent_runs, 2);
        assert_eq!(agg.total_tool_invocations, 4); // 2 + 1 from agent runs + 1 from tool event
    }

    #[test]
    fn drain_period_returns_and_clears() {
        let (mut svc, _) = make_service();

        svc.record_event(agent_run_event("user-1", 100, 50, 0, 1000)).unwrap();
        svc.record_event(agent_run_event("user-2", 200, 80, 0, 1100)).unwrap();

        let drained = svc.drain_period(2000);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].user_id, "user-1");
        assert_eq!(drained[1].user_id, "user-2");
        assert!(drained.iter().all(|a| a.period_end == 2000));

        // Counters are cleared
        assert!(svc.counters().is_empty());
    }

    #[test]
    fn get_usage_respects_time_range() {
        let (mut svc, _) = make_service();
        svc.record_event(agent_run_event("user-1", 100, 50, 0, 1000)).unwrap();

        // Within range
        assert!(svc.get_usage("user-1", 500, 1500).is_some());
        // Outside range
        assert!(svc.get_usage("user-1", 2000, 3000).is_none());
    }

    #[test]
    fn get_team_usage_aggregates_across_users() {
        let (mut svc, _) = make_service();

        let mut e1 = agent_run_event("user-1", 100, 50, 1, 1000);
        e1.team_id = Some("team-a".to_string());
        svc.record_event(e1).unwrap();

        let mut e2 = agent_run_event("user-2", 200, 80, 2, 1100);
        e2.team_id = Some("team-a".to_string());
        svc.record_event(e2).unwrap();

        let team_agg = svc.get_team_usage("team-a", 0, 2000).unwrap();
        assert_eq!(team_agg.total_input_tokens, 300);
        assert_eq!(team_agg.total_output_tokens, 130);
        assert_eq!(team_agg.total_agent_runs, 2);
        assert_eq!(team_agg.total_tool_invocations, 3);

        // Non-existent team
        assert!(svc.get_team_usage("team-b", 0, 2000).is_none());
    }

    #[test]
    fn nonexistent_user_returns_none() {
        let (svc, _) = make_service();
        assert!(svc.get_usage("nobody", 0, 9999).is_none());
    }
}
