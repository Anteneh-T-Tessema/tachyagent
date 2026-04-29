use serde::{Deserialize, Serialize};

use crate::agent::AgentConfig;

/// When a scheduled task should run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRule {
    /// Run once immediately.
    Once,
    /// Run on a cron-like schedule (simplified: interval in seconds).
    Interval { seconds: u64 },
    /// Run when a file matching the pattern changes.
    OnFileChange { patterns: Vec<String> },
    /// Run on webhook trigger.
    OnWebhook { path: String },
    /// Run on git events (push, PR, etc.).
    OnGitEvent { event: String },
}

/// Status of a scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// A task that runs an agent on a schedule or trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub agent_config: AgentConfig,
    pub schedule: ScheduleRule,
    pub status: TaskStatus,
    pub run_count: u32,
    pub last_run_at: Option<String>,
    pub enabled: bool,
}

impl ScheduledTask {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        agent_config: AgentConfig,
        schedule: ScheduleRule,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            agent_config,
            schedule,
            status: TaskStatus::Pending,
            run_count: 0,
            last_run_at: None,
            enabled: true,
        }
    }

    pub fn record_run(&mut self, success: bool) {
        self.run_count += 1;
        self.last_run_at = Some(timestamp());
        self.status = if success {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
    }
}

/// Manages scheduled tasks and their lifecycle.
pub struct TaskScheduler {
    tasks: Vec<ScheduledTask>,
}

impl TaskScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn add_task(&mut self, task: ScheduledTask) {
        self.tasks.push(task);
    }

    pub fn remove_task(&mut self, id: &str) -> bool {
        let len_before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        self.tasks.len() < len_before
    }

    #[must_use]
    pub fn list_tasks(&self) -> &[ScheduledTask] {
        &self.tasks
    }

    #[must_use]
    pub fn get_task(&self, id: &str) -> Option<&ScheduledTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn get_task_mut(&mut self, id: &str) -> Option<&mut ScheduledTask> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    /// Return tasks that are due to run (simplified: all enabled pending/completed interval tasks).
    #[must_use]
    pub fn due_tasks(&self) -> Vec<&ScheduledTask> {
        self.tasks
            .iter()
            .filter(|t| {
                t.enabled
                    && matches!(
                        (&t.schedule, t.status),
                        (
                            ScheduleRule::Once
                                | ScheduleRule::OnFileChange { .. }
                                | ScheduleRule::OnWebhook { .. }
                                | ScheduleRule::OnGitEvent { .. },
                            TaskStatus::Pending
                        ) | (
                            ScheduleRule::Interval { .. },
                            TaskStatus::Pending | TaskStatus::Completed
                        )
                    )
            })
            .collect()
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        Self::new()
    }
}

fn timestamp() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s", d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentConfig, AgentTemplate};

    fn test_config() -> AgentConfig {
        AgentConfig {
            template: AgentTemplate::test_runner(),
            session_id: "s1".to_string(),
            working_directory: "/tmp".to_string(),
            environment: std::collections::BTreeMap::default(),
            team_id: None,
        }
    }

    #[test]
    fn scheduler_manages_tasks() {
        let mut scheduler = TaskScheduler::new();

        scheduler.add_task(ScheduledTask::new(
            "t1",
            "Run tests",
            test_config(),
            ScheduleRule::Once,
        ));
        scheduler.add_task(ScheduledTask::new(
            "t2",
            "Lint on change",
            test_config(),
            ScheduleRule::OnFileChange {
                patterns: vec!["**/*.rs".to_string()],
            },
        ));

        assert_eq!(scheduler.list_tasks().len(), 2);
        assert_eq!(scheduler.due_tasks().len(), 2);

        // Complete a task
        scheduler.get_task_mut("t1").unwrap().record_run(true);
        // Once tasks don't re-run after completion
        let due = scheduler.due_tasks();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "t2");

        // Remove a task
        assert!(scheduler.remove_task("t2"));
        assert_eq!(scheduler.list_tasks().len(), 1);
    }

    #[test]
    fn interval_tasks_re_run_after_completion() {
        let mut scheduler = TaskScheduler::new();
        scheduler.add_task(ScheduledTask::new(
            "t1",
            "Periodic scan",
            test_config(),
            ScheduleRule::Interval { seconds: 3600 },
        ));

        assert_eq!(scheduler.due_tasks().len(), 1);
        scheduler.get_task_mut("t1").unwrap().record_run(true);
        // Interval tasks are due again after completion
        assert_eq!(scheduler.due_tasks().len(), 1);
    }
}
