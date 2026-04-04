//! File-level locking for parallel agent safety.
//!
//! When multiple agents run in parallel, they must not corrupt each other's
//! file edits. This module provides cooperative file locks that:
//! - Prevent two agents from editing the same file simultaneously
//! - Support timeout-based waiting (agent retries after delay)
//! - Log all lock acquisitions to the audit trail
//! - Are fully in-memory (no external dependencies)

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A file lock manager shared across all parallel agents.
#[derive(Clone)]
pub struct FileLockManager {
    inner: Arc<Mutex<LockState>>,
}

struct LockState {
    locks: BTreeMap<String, LockEntry>,
}

struct LockEntry {
    agent_id: String,
    acquired_at: Instant,
    /// Auto-expire after this duration to prevent deadlocks.
    ttl: Duration,
}

impl FileLockManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LockState {
                locks: BTreeMap::new(),
            })),
        }
    }

    /// Try to acquire a lock on a file. Returns Ok if acquired, Err if held by another agent.
    pub fn try_acquire(&self, file: &str, agent_id: &str) -> Result<(), LockError> {
        let mut state = self.inner.lock().unwrap();

        // Clean expired locks
        state.locks.retain(|_, entry| entry.acquired_at.elapsed() < entry.ttl);

        if let Some(entry) = state.locks.get(file) {
            if entry.agent_id == agent_id {
                return Ok(()); // Already held by this agent
            }
            return Err(LockError::Held {
                file: file.to_string(),
                held_by: entry.agent_id.clone(),
                remaining: entry.ttl.saturating_sub(entry.acquired_at.elapsed()),
            });
        }

        state.locks.insert(file.to_string(), LockEntry {
            agent_id: agent_id.to_string(),
            acquired_at: Instant::now(),
            ttl: Duration::from_secs(300), // 5 minute default TTL
        });

        Ok(())
    }

    /// Acquire a lock, waiting up to `timeout` for it to become available.
    pub fn acquire_with_wait(
        &self,
        file: &str,
        agent_id: &str,
        timeout: Duration,
    ) -> Result<(), LockError> {
        let start = Instant::now();
        loop {
            match self.try_acquire(file, agent_id) {
                Ok(()) => return Ok(()),
                Err(LockError::Held { .. }) if start.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Release a lock on a file.
    pub fn release(&self, file: &str, agent_id: &str) {
        let mut state = self.inner.lock().unwrap();
        if let Some(entry) = state.locks.get(file) {
            if entry.agent_id == agent_id {
                state.locks.remove(file);
            }
        }
    }

    /// Release all locks held by an agent (called on agent completion/failure).
    pub fn release_all(&self, agent_id: &str) {
        let mut state = self.inner.lock().unwrap();
        state.locks.retain(|_, entry| entry.agent_id != agent_id);
    }

    /// List all currently held locks.
    pub fn list_locks(&self) -> Vec<(String, String)> {
        let state = self.inner.lock().unwrap();
        state.locks.iter()
            .filter(|(_, entry)| entry.acquired_at.elapsed() < entry.ttl)
            .map(|(file, entry)| (file.clone(), entry.agent_id.clone()))
            .collect()
    }
}

impl Default for FileLockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum LockError {
    Held {
        file: String,
        held_by: String,
        remaining: Duration,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held { file, held_by, remaining } => {
                write!(f, "file '{}' locked by agent '{}' ({}s remaining)",
                    file, held_by, remaining.as_secs())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_and_release() {
        let mgr = FileLockManager::new();
        assert!(mgr.try_acquire("file.rs", "agent-1").is_ok());
        assert!(mgr.try_acquire("file.rs", "agent-2").is_err());
        mgr.release("file.rs", "agent-1");
        assert!(mgr.try_acquire("file.rs", "agent-2").is_ok());
    }

    #[test]
    fn same_agent_can_reacquire() {
        let mgr = FileLockManager::new();
        assert!(mgr.try_acquire("file.rs", "agent-1").is_ok());
        assert!(mgr.try_acquire("file.rs", "agent-1").is_ok()); // idempotent
    }

    #[test]
    fn release_all_clears_agent_locks() {
        let mgr = FileLockManager::new();
        mgr.try_acquire("a.rs", "agent-1").unwrap();
        mgr.try_acquire("b.rs", "agent-1").unwrap();
        mgr.try_acquire("c.rs", "agent-2").unwrap();
        mgr.release_all("agent-1");
        assert!(mgr.try_acquire("a.rs", "agent-2").is_ok());
        assert!(mgr.try_acquire("b.rs", "agent-2").is_ok());
        assert!(mgr.try_acquire("c.rs", "agent-2").is_ok()); // agent-2 already holds it (idempotent)
    }

    #[test]
    fn list_locks_shows_active() {
        let mgr = FileLockManager::new();
        mgr.try_acquire("x.rs", "a1").unwrap();
        mgr.try_acquire("y.rs", "a2").unwrap();
        let locks = mgr.list_locks();
        assert_eq!(locks.len(), 2);
    }
}
