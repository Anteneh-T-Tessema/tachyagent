//! Distributed swarm worker registry.
//!
//! In single-machine mode (default), all workers are local threads managed by
//! the `parallel` module. In distributed mode, remote Tachy daemons running
//! with `--worker-mode` register here and receive task assignments via HTTP.
//!
//! Architecture:
//!   Coordinator daemon      Worker daemons (any machine)
//!   ┌──────────────┐        ┌─────────────────────────┐
//!   │ `WorkerRegistry`│◄──────│ POST /api/workers/register│
//!   │  (this module)│       │ POST /api/workers/heartbeat│
//!   │  `dispatch()`   │──────►│ POST /api/tasks/assign   │
//!   └──────────────┘        └─────────────────────────┘

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

/// A registered remote worker daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    /// Unique worker ID (daemon generates on first registration).
    pub id: String,
    /// Human-readable hostname or IP.
    pub host: String,
    /// HTTP URL of the worker's daemon (e.g. <http://10.0.0.5:7777>).
    pub url: String,
    /// Maximum concurrent tasks this worker supports.
    pub max_concurrency: usize,
    /// Currently active task count (reported by heartbeat).
    pub active_tasks: usize,
    /// Epoch seconds of last heartbeat.
    pub last_seen: u64,
    /// Worker capabilities (e.g. GPU, RAM tier).
    #[serde(default)]
    pub tags: Vec<String>,
}

impl WorkerNode {
    /// True if the worker has been silent for more than `timeout_secs`.
    #[must_use] pub fn is_stale(&self, timeout_secs: u64) -> bool {
        now_epoch().saturating_sub(self.last_seen) > timeout_secs
    }

    /// Available capacity (`max_concurrency` - `active_tasks`).
    #[must_use] pub fn available_slots(&self) -> usize {
        self.max_concurrency.saturating_sub(self.active_tasks)
    }
}

/// The registry of all known remote workers.
#[derive(Debug, Default)]
pub struct WorkerRegistry {
    workers: BTreeMap<String, WorkerNode>,
    /// Heartbeat timeout: workers not seen for this many seconds are considered dead.
    heartbeat_timeout_secs: u64,
}

impl WorkerRegistry {
    #[must_use] pub fn new() -> Self {
        Self {
            workers: BTreeMap::new(),
            heartbeat_timeout_secs: 30,
        }
    }

    /// Register or refresh a worker. Returns the worker ID.
    pub fn register(&mut self, mut worker: WorkerNode) -> String {
        worker.last_seen = now_epoch();
        let id = worker.id.clone();
        eprintln!("[registry] worker registered: {id} @ {} ({} slots)", worker.url, worker.max_concurrency);
        self.workers.insert(id.clone(), worker);
        id
    }

    /// Update a worker's heartbeat and active task count.
    pub fn heartbeat(&mut self, worker_id: &str, active_tasks: usize) -> bool {
        if let Some(w) = self.workers.get_mut(worker_id) {
            w.last_seen = now_epoch();
            w.active_tasks = active_tasks;
            true
        } else {
            false
        }
    }

    /// Remove a worker from the registry.
    pub fn deregister(&mut self, worker_id: &str) {
        self.workers.remove(worker_id);
        eprintln!("[registry] worker deregistered: {worker_id}");
    }

    /// Prune workers that haven't sent a heartbeat within the timeout.
    pub fn prune_stale(&mut self) {
        let timeout = self.heartbeat_timeout_secs;
        let before = self.workers.len();
        self.workers.retain(|_, w| !w.is_stale(timeout));
        let pruned = before - self.workers.len();
        if pruned > 0 {
            eprintln!("[registry] pruned {pruned} stale workers");
        }
    }

    /// Pick the least-loaded available worker.
    #[must_use] pub fn pick_worker(&self) -> Option<&WorkerNode> {
        self.workers.values()
            .filter(|w| !w.is_stale(self.heartbeat_timeout_secs) && w.available_slots() > 0)
            .max_by_key(|w| w.available_slots())
    }

    /// Dispatch a task to the least-loaded worker via HTTP.
    /// Falls back to `None` if no workers are available (caller uses local execution).
    #[must_use] pub fn dispatch_task(&self, task_json: &str) -> Option<String> {
        let worker = self.pick_worker()?;
        let url = format!("{}/api/tasks/assign", worker.url);

        let resp = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .ok()?
            .post(&url)
            .header("Content-Type", "application/json")
            .body(task_json.to_string())
            .send()
            .ok()?;

        if resp.status().is_success() {
            let body: serde_json::Value = resp.json().ok()?;
            eprintln!("[registry] task dispatched to worker {}: {}", worker.id, body);
            Some(worker.id.clone())
        } else {
            eprintln!("[registry] dispatch to {} failed: {}", worker.id, resp.status());
            None
        }
    }

    #[must_use] pub fn list_workers(&self) -> Vec<&WorkerNode> {
        self.workers.values().collect()
    }

    #[must_use] pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    #[must_use] pub fn available_worker_count(&self) -> usize {
        let timeout = self.heartbeat_timeout_secs;
        self.workers.values()
            .filter(|w| !w.is_stale(timeout) && w.available_slots() > 0)
            .count()
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_worker(id: &str, url: &str, max: usize, active: usize) -> WorkerNode {
        WorkerNode {
            id: id.to_string(),
            host: "localhost".to_string(),
            url: url.to_string(),
            max_concurrency: max,
            active_tasks: active,
            last_seen: now_epoch(),
            tags: vec![],
        }
    }

    #[test]
    fn register_and_list() {
        let mut reg = WorkerRegistry::new();
        reg.register(make_worker("w1", "http://10.0.0.1:7777", 4, 0));
        reg.register(make_worker("w2", "http://10.0.0.2:7777", 8, 2));
        assert_eq!(reg.worker_count(), 2);
        assert_eq!(reg.available_worker_count(), 2);
    }

    #[test]
    fn picks_least_loaded_worker() {
        let mut reg = WorkerRegistry::new();
        reg.register(make_worker("w1", "http://a:7777", 4, 3)); // 1 slot
        reg.register(make_worker("w2", "http://b:7777", 4, 0)); // 4 slots
        let picked = reg.pick_worker().unwrap();
        assert_eq!(picked.id, "w2");
    }

    #[test]
    fn stale_worker_excluded_from_dispatch() {
        let mut reg = WorkerRegistry::new();
        let mut stale = make_worker("w_stale", "http://c:7777", 4, 0);
        stale.last_seen = 0; // way in the past
        reg.workers.insert(stale.id.clone(), stale);
        assert!(reg.pick_worker().is_none());
    }

    #[test]
    fn heartbeat_refreshes_last_seen() {
        let mut reg = WorkerRegistry::new();
        reg.register(make_worker("w1", "http://a:7777", 4, 2));
        let ok = reg.heartbeat("w1", 1);
        assert!(ok);
        let w = &reg.workers["w1"];
        assert_eq!(w.active_tasks, 1);
        assert!(!w.is_stale(30));
    }

    #[test]
    fn prune_removes_stale() {
        let mut reg = WorkerRegistry::new();
        reg.register(make_worker("live", "http://a:7777", 4, 0));
        let mut stale = make_worker("dead", "http://b:7777", 4, 0);
        stale.last_seen = 0;
        reg.workers.insert(stale.id.clone(), stale);
        reg.prune_stale();
        assert_eq!(reg.worker_count(), 1);
        assert!(reg.workers.contains_key("live"));
    }

    #[test]
    fn deregister_removes_worker() {
        let mut reg = WorkerRegistry::new();
        reg.register(make_worker("w1", "http://a:7777", 4, 0));
        reg.deregister("w1");
        assert_eq!(reg.worker_count(), 0);
    }
}
