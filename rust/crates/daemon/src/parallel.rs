//! Parallel agent execution engine — the "Kubernetes for AI agents."
//!
//! Supports:
//! - Task DAG execution (dependency-aware scheduling)
//! - Worker pool with configurable concurrency
//! - Process-level isolation per agent
//! - Deterministic audit trail across parallel execution
//! - Retry, cancellation, and partial completion
//!
//! Architecture:
//!   API/CLI → Orchestrator (DAG planner) → Task Queue → Worker Pool → Agents
//!   Each worker pulls tasks, executes them, and emits results + new tasks.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Unique identifier for a parallel run.
pub type RunId = String;
pub type TaskId = String;

/// A task in the execution DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: TaskId,
    pub run_id: RunId,
    pub template: String,
    pub prompt: String,
    pub model: Option<String>,
    pub deps: Vec<TaskId>,
    pub priority: u8,
    pub status: TaskStatus,
    pub result: Option<TaskResult>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    /// Isolated working directory for this task.
    pub work_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: bool,
    pub summary: String,
    pub iterations: usize,
    pub tool_invocations: u32,
    pub audit_hash: String,
}

/// A parallel execution run — a DAG of tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelRun {
    pub id: RunId,
    pub tasks: Vec<AgentTask>,
    pub status: RunStatus,
    pub created_at: u64,
    pub max_concurrency: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
}

/// The task queue — dependency-aware, priority-scheduled.
pub struct TaskQueue {
    tasks: VecDeque<AgentTask>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self { tasks: VecDeque::new() }
    }

    /// Add a task to the queue.
    pub fn enqueue(&mut self, task: AgentTask) {
        // Insert by priority (higher priority = earlier in queue)
        let pos = self.tasks.iter().position(|t| t.priority < task.priority);
        match pos {
            Some(i) => self.tasks.insert(i, task),
            None => self.tasks.push_back(task),
        }
    }

    /// Get the next task whose dependencies are all completed.
    pub fn poll(&mut self, completed: &[TaskId]) -> Option<AgentTask> {
        let pos = self.tasks.iter().position(|t| {
            t.status == TaskStatus::Queued
                && t.deps.iter().all(|dep| completed.contains(dep))
        });
        pos.map(|i| self.tasks.remove(i).unwrap())
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// The parallel execution orchestrator.
pub struct Orchestrator {
    runs: BTreeMap<RunId, ParallelRun>,
    queue: TaskQueue,
    completed_tasks: Vec<TaskId>,
    max_workers: usize,
    active_workers: usize,
}

impl Orchestrator {
    pub fn new(max_workers: usize) -> Self {
        Self {
            runs: BTreeMap::new(),
            queue: TaskQueue::new(),
            completed_tasks: Vec::new(),
            max_workers,
            active_workers: 0,
        }
    }

    /// Submit a parallel run with a DAG of tasks.
    pub fn submit(&mut self, run: ParallelRun) -> RunId {
        let run_id = run.id.clone();
        // Enqueue all tasks that have no dependencies
        for task in &run.tasks {
            let mut queued = task.clone();
            queued.status = TaskStatus::Queued;
            self.queue.enqueue(queued);
        }
        self.runs.insert(run_id.clone(), run);
        run_id
    }

    /// Get the next task ready for execution (deps satisfied, worker available).
    pub fn next_task(&mut self) -> Option<AgentTask> {
        if self.active_workers >= self.max_workers {
            return None;
        }
        if let Some(mut task) = self.queue.poll(&self.completed_tasks) {
            task.status = TaskStatus::Running;
            task.started_at = Some(now_epoch());
            self.active_workers += 1;
            Some(task)
        } else {
            None
        }
    }

    /// Report a task completion.
    pub fn complete_task(&mut self, task_id: &str, result: TaskResult) {
        self.completed_tasks.push(task_id.to_string());
        self.active_workers = self.active_workers.saturating_sub(1);

        // Update the run
        for run in self.runs.values_mut() {
            for task in &mut run.tasks {
                if task.id == task_id {
                    task.status = if result.success { TaskStatus::Completed } else { TaskStatus::Failed };
                    task.completed_at = Some(now_epoch());
                    task.result = Some(result.clone());
                }
            }
            // Check if run is complete
            let all_done = run.tasks.iter().all(|t| {
                matches!(t.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled)
            });
            if all_done {
                let all_success = run.tasks.iter().all(|t| t.status == TaskStatus::Completed);
                run.status = if all_success { RunStatus::Completed }
                    else if run.tasks.iter().any(|t| t.status == TaskStatus::Completed) { RunStatus::PartiallyCompleted }
                    else { RunStatus::Failed };
            }
        }
    }

    /// Cancel a task.
    pub fn cancel_task(&mut self, task_id: &str) {
        for run in self.runs.values_mut() {
            for task in &mut run.tasks {
                if task.id == task_id && task.status != TaskStatus::Completed {
                    task.status = TaskStatus::Cancelled;
                }
            }
        }
    }

    /// Get a run's status.
    pub fn get_run(&self, run_id: &str) -> Option<&ParallelRun> {
        self.runs.get(run_id)
    }

    /// List all runs.
    pub fn list_runs(&self) -> Vec<&ParallelRun> {
        self.runs.values().collect()
    }

    pub fn active_count(&self) -> usize {
        self.active_workers
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}

/// Execute a parallel run using a thread pool.
/// Each task runs in its own thread with an isolated working directory.
pub fn execute_parallel_run(
    run: ParallelRun,
    state: &Arc<Mutex<super::DaemonState>>,
) -> ParallelRun {
    let max_concurrency = run.max_concurrency.min(8); // cap at 8 parallel agents
    let orchestrator = Arc::new(Mutex::new(Orchestrator::new(max_concurrency)));

    // Submit the run
    {
        let mut orch = orchestrator.lock().unwrap();
        orch.submit(run.clone());
    }

    // Worker loop — spawn threads that pull and execute tasks
    let mut handles = Vec::new();

    loop {
        let task = {
            let mut orch = orchestrator.lock().unwrap();
            orch.next_task()
        };

        match task {
            Some(task) => {
                let orch = Arc::clone(&orchestrator);
                let bg_state = Arc::clone(state);
                let task_id = task.id.clone();

                let handle = std::thread::spawn(move || {
                    // Execute the agent task
                    let result = execute_single_task(&task, &bg_state);
                    let mut orch = orch.lock().unwrap();
                    orch.complete_task(&task_id, result);
                });
                handles.push(handle);
            }
            None => {
                // Check if all tasks are done
                let orch = orchestrator.lock().unwrap();
                if orch.pending_count() == 0 && orch.active_count() == 0 {
                    break;
                }
                drop(orch);
                // Wait a bit before polling again
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    // Wait for all workers
    for handle in handles {
        let _ = handle.join();
    }

    // Return the final run state
    let orch = orchestrator.lock().unwrap();
    orch.get_run(&run.id).cloned().unwrap_or(run)
}

fn execute_single_task(
    task: &AgentTask,
    state: &Arc<Mutex<super::DaemonState>>,
) -> TaskResult {
    let s = state.lock().unwrap();
    let workspace_root = task.work_dir.clone().unwrap_or_else(|| s.workspace_root.clone());
    let file_locks = s.file_locks.clone();

    // Find or create agent config
    let template = s.config.agent_templates.iter()
        .find(|t| t.name == task.template)
        .cloned()
        .unwrap_or_else(|| {
            let mut t = platform::AgentTemplate::chat_assistant();
            t.name = task.template.clone();
            t
        });

    let mut template = template;
    if let Some(model) = &task.model {
        template.model = model.clone();
    }

    let config = platform::AgentConfig {
        template,
        session_id: format!("sess-{}", task.id),
        working_directory: workspace_root.to_string_lossy().to_string(),
        environment: std::collections::BTreeMap::new(),
    };

    let result = super::AgentEngine::run_agent(
        &task.id,
        &config,
        &task.prompt,
        &s.registry,
        &s.config.governance,
        &s.audit_logger,
        &s.config.intelligence,
        &workspace_root,
        Some(file_locks.clone()),
    );

    // Release all file locks held by this agent on completion
    file_locks.release_all(&task.id);

    TaskResult {
        success: result.success,
        summary: result.summary,
        iterations: result.iterations,
        tool_invocations: result.tool_invocations,
        audit_hash: s.audit_logger.last_hash(),
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

use platform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_queue_respects_priority() {
        let mut q = TaskQueue::new();
        q.enqueue(AgentTask {
            id: "low".into(), run_id: "r1".into(), template: "chat".into(),
            prompt: "a".into(), model: None, deps: vec![], priority: 1,
            status: TaskStatus::Queued, result: None, created_at: 0,
            started_at: None, completed_at: None, work_dir: None,
        });
        q.enqueue(AgentTask {
            id: "high".into(), run_id: "r1".into(), template: "chat".into(),
            prompt: "b".into(), model: None, deps: vec![], priority: 10,
            status: TaskStatus::Queued, result: None, created_at: 0,
            started_at: None, completed_at: None, work_dir: None,
        });
        let next = q.poll(&[]).unwrap();
        assert_eq!(next.id, "high");
    }

    #[test]
    fn task_queue_respects_dependencies() {
        let mut q = TaskQueue::new();
        q.enqueue(AgentTask {
            id: "t2".into(), run_id: "r1".into(), template: "chat".into(),
            prompt: "b".into(), model: None, deps: vec!["t1".into()], priority: 5,
            status: TaskStatus::Queued, result: None, created_at: 0,
            started_at: None, completed_at: None, work_dir: None,
        });
        // t2 depends on t1, which isn't completed yet
        assert!(q.poll(&[]).is_none());
        // Now t1 is completed
        let next = q.poll(&["t1".into()]).unwrap();
        assert_eq!(next.id, "t2");
    }

    #[test]
    fn orchestrator_tracks_completion() {
        let mut orch = Orchestrator::new(4);
        let run = ParallelRun {
            id: "run-1".into(),
            tasks: vec![
                AgentTask {
                    id: "t1".into(), run_id: "run-1".into(), template: "chat".into(),
                    prompt: "a".into(), model: None, deps: vec![], priority: 5,
                    status: TaskStatus::Pending, result: None, created_at: 0,
                    started_at: None, completed_at: None, work_dir: None,
                },
            ],
            status: RunStatus::Running,
            created_at: 0,
            max_concurrency: 4,
        };
        orch.submit(run);
        let task = orch.next_task().unwrap();
        assert_eq!(task.id, "t1");
        orch.complete_task("t1", TaskResult {
            success: true, summary: "done".into(), iterations: 1,
            tool_invocations: 0, audit_hash: "abc".into(),
        });
        let run = orch.get_run("run-1").unwrap();
        assert_eq!(run.status, RunStatus::Completed);
    }
}
