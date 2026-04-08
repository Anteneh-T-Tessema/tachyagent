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
    /// Can define and manage templates and swarms. Can run agents.
    Architect,
    /// Full access — can manage users, change governance, run agents, view audit.
    Admin,
    /// Highest privilege — can manage SSO, review full audit chains, and set security policies.
    SecurityAdmin,
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
    BroadcastEvent,
    ViewMissionFeed,
    ManageUsers,
    ManageGovernance,
    ManageConfig,
    ManageEnterpriseSSO,
}

/// Check if a role is allowed to perform an action.
#[must_use] pub fn check_permission(role: Role, action: Action) -> AccessResult {
    let allowed = match action {
        // Everyone can view health and models
        Action::ViewHealth | Action::ListModels | Action::ListTemplates => true,
        // Viewers can see agents, tasks, and audit
        Action::ListAgents | Action::ListTasks | Action::ViewAudit => true,
        // Broadcast and Feed accessible to Developers and above
        Action::BroadcastEvent | Action::ViewMissionFeed => role >= Role::Developer,
        // Developers and above can run agents and schedule tasks
        Action::RunAgent | Action::ScheduleTask => role >= Role::Developer,
        // Architects and above can manage templates and swarms (ManageConfig)
        Action::ManageConfig => role >= Role::Architect,
        // Only admins and above can manage users and governance
        Action::ManageUsers | Action::ManageGovernance => role >= Role::Admin,
        // Only security admins can manage SSO
        Action::ManageEnterpriseSSO => role >= Role::SecurityAdmin,
    };

    if allowed {
        AccessResult::Allowed
    } else {
        AccessResult::Denied {
            reason: format!("{role:?} cannot perform {action:?}"),
        }
    }
}

/// Check if a user is allowed to perform an action within a specific team.
///
/// The caller must resolve the user's role in the team (via `TeamManager::get_member_role`)
/// and pass it as `team_role`. If the user is not a member, pass `None`.
#[must_use] pub fn check_team_permission(
    user_id: &str,
    team_id: &str,
    action: Action,
    team_role: Option<Role>,
) -> AccessResult {
    match team_role {
        Some(role) => check_permission(role, action),
        None => AccessResult::Denied {
            reason: format!("user '{user_id}' is not a member of team '{team_id}'"),
        },
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
    #[must_use] pub fn with_default_admin(api_key_hash: &str) -> Self {
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
    #[must_use] pub fn authenticate(&self, api_key_hash: &str) -> Option<&User> {
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
    #[must_use] pub fn list_users(&self) -> Vec<&User> {
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

    #[test]
    fn team_permission_allows_member_with_role() {
        let result = check_team_permission("user-1", "team-1", Action::RunAgent, Some(Role::Developer));
        assert_eq!(result, AccessResult::Allowed);
    }

    #[test]
    fn team_permission_denies_viewer_running_agent() {
        let result = check_team_permission("user-1", "team-1", Action::RunAgent, Some(Role::Viewer));
        assert!(matches!(result, AccessResult::Denied { .. }));
    }

    #[test]
    fn team_permission_denies_non_member() {
        let result = check_team_permission("user-1", "team-1", Action::ListAgents, None);
        assert!(matches!(result, AccessResult::Denied { ref reason } if reason.contains("not a member")));
    }

    #[test]
    fn team_permission_admin_can_manage() {
        let result = check_team_permission("user-1", "team-1", Action::ManageUsers, Some(Role::Admin));
        assert_eq!(result, AccessResult::Allowed);
    }
}
