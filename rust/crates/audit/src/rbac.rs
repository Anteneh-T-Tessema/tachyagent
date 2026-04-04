//! Role-Based Access Control (RBAC) for the Tachy platform.
//! Defines roles, permissions, and user management.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// User roles with increasing privilege levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Can view agent results and audit logs. Cannot run agents or change config.
    Viewer,
    /// Can run agents and view results. Cannot change governance or manage users.
    Developer,
    /// Full access — can manage users, change governance, run agents, view audit.
    Admin,
}

/// A user in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// API key hash (not the raw key)
    pub api_key_hash: String,
    pub created_at: String,
    pub enabled: bool,
}

/// Permission check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessResult {
    Allowed,
    Denied { reason: String },
}

/// What actions each role can perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    ViewHealth,
    ListModels,
    ListTemplates,
    ListAgents,
    ListTasks,
    RunAgent,
    ScheduleTask,
    ViewAudit,
    ManageUsers,
    ManageGovernance,
    ManageConfig,
}

/// Check if a role is allowed to perform an action.
pub fn check_permission(role: Role, action: Action) -> AccessResult {
    let allowed = match action {
        // Everyone can view health and models
        Action::ViewHealth | Action::ListModels | Action::ListTemplates => true,
        // Viewers can see agents, tasks, and audit
        Action::ListAgents | Action::ListTasks | Action::ViewAudit => true,
        // Developers and above can run agents and schedule tasks
        Action::RunAgent | Action::ScheduleTask => role >= Role::Developer,
        // Only admins can manage users, governance, and config
        Action::ManageUsers | Action::ManageGovernance | Action::ManageConfig => role == Role::Admin,
    };

    if allowed {
        AccessResult::Allowed
    } else {
        AccessResult::Denied {
            reason: format!("{role:?} cannot perform {action:?}"),
        }
    }
}

/// User store — manages users and API key authentication.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserStore {
    pub users: BTreeMap<String, User>,
}

impl UserStore {
    #[must_use]
    pub fn new() -> Self {
        Self { users: BTreeMap::new() }
    }

    /// Create a default admin user.
    pub fn with_default_admin(api_key_hash: &str) -> Self {
        let mut store = Self::new();
        store.users.insert("admin".to_string(), User {
            id: "admin".to_string(),
            name: "Default Admin".to_string(),
            role: Role::Admin,
            api_key_hash: api_key_hash.to_string(),
            created_at: timestamp(),
            enabled: true,
        });
        store
    }

    /// Authenticate a user by API key hash. Returns the user if found and enabled.
    pub fn authenticate(&self, api_key_hash: &str) -> Option<&User> {
        self.users.values().find(|u| u.enabled && u.api_key_hash == api_key_hash)
    }

    /// Add a new user.
    pub fn add_user(&mut self, user: User) {
        self.users.insert(user.id.clone(), user);
    }

    /// Remove a user.
    pub fn remove_user(&mut self, id: &str) -> bool {
        self.users.remove(id).is_some()
    }

    /// List all users.
    pub fn list_users(&self) -> Vec<&User> {
        self.users.values().collect()
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

    #[test]
    fn viewer_cannot_run_agents() {
        assert_eq!(check_permission(Role::Viewer, Action::RunAgent), AccessResult::Denied { reason: "Viewer cannot perform RunAgent".to_string() });
    }

    #[test]
    fn developer_can_run_agents() {
        assert_eq!(check_permission(Role::Developer, Action::RunAgent), AccessResult::Allowed);
    }

    #[test]
    fn developer_cannot_manage_users() {
        assert!(matches!(check_permission(Role::Developer, Action::ManageUsers), AccessResult::Denied { .. }));
    }

    #[test]
    fn admin_can_do_everything() {
        for action in [Action::ViewHealth, Action::RunAgent, Action::ManageUsers, Action::ManageGovernance] {
            assert_eq!(check_permission(Role::Admin, action), AccessResult::Allowed);
        }
    }

    #[test]
    fn user_store_authentication() {
        let store = UserStore::with_default_admin("hash123");
        assert!(store.authenticate("hash123").is_some());
        assert!(store.authenticate("wrong").is_none());
    }

    #[test]
    fn user_store_crud() {
        let mut store = UserStore::new();
        store.add_user(User {
            id: "dev1".to_string(),
            name: "Developer".to_string(),
            role: Role::Developer,
            api_key_hash: "abc".to_string(),
            created_at: "now".to_string(),
            enabled: true,
        });
        assert_eq!(store.list_users().len(), 1);
        assert!(store.remove_user("dev1"));
        assert_eq!(store.list_users().len(), 0);
    }

    #[test]
    fn disabled_user_cannot_authenticate() {
        let mut store = UserStore::new();
        store.add_user(User {
            id: "disabled".to_string(),
            name: "Disabled".to_string(),
            role: Role::Admin,
            api_key_hash: "key".to_string(),
            created_at: "now".to_string(),
            enabled: false,
        });
        assert!(store.authenticate("key").is_none());
    }
}
