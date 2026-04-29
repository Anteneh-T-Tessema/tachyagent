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
    /// The currently active team for this user session.
    pub active_team_id: Option<String>,
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
    ManageWebhooks,
    ManageCloudJobs,
    ManageModels,
    ManageIntelligence,
    ManagePolicies,
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
        // ManageModels, Webhooks, Intelligence, and Policies require Architect or above
        Action::ManageModels | Action::ManageWebhooks | Action::ManageCloudJobs | Action::ManageIntelligence | Action::ManagePolicies => role >= Role::Architect,
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
            active_team_id: None,
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

/// Per-role resource quotas for token and cost limits.
///
/// Zero values mean unlimited. These defaults are calibrated for regulated-enterprise
/// use where a rogue agent draining the team budget is a compliance incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleQuota {
    /// Max tokens per hour across all tasks for this user (0 = unlimited).
    pub max_tokens_per_hour: u64,
    /// Max cost in USD per day (0.0 = unlimited).
    pub max_cost_usd_per_day: f64,
    /// Max simultaneous agent runs (0 = unlimited).
    pub max_concurrent_runs: u32,
}

impl Default for RoleQuota {
    fn default() -> Self {
        Self { max_tokens_per_hour: 0, max_cost_usd_per_day: 0.0, max_concurrent_runs: 0 }
    }
}

/// Conservative defaults per role for regulated environments.
#[must_use]
pub fn default_quota_for_role(role: Role) -> RoleQuota {
    match role {
        Role::Viewer      => RoleQuota { max_tokens_per_hour: 10_000,  max_cost_usd_per_day: 0.10, max_concurrent_runs: 1  },
        Role::Developer   => RoleQuota { max_tokens_per_hour: 100_000, max_cost_usd_per_day: 5.0,  max_concurrent_runs: 3  },
        Role::Architect   => RoleQuota { max_tokens_per_hour: 500_000, max_cost_usd_per_day: 25.0, max_concurrent_runs: 10 },
        Role::Admin        | Role::SecurityAdmin
                          => RoleQuota { max_tokens_per_hour: 0, max_cost_usd_per_day: 0.0, max_concurrent_runs: 0 },
    }
}

/// Sliding-window usage counters for a single user.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserUsage {
    pub user_id: String,
    pub tokens_this_hour: u64,
    pub cost_usd_today: f64,
    pub active_runs: u32,
    /// Unix-second start of the current hourly window.
    pub window_start_secs: u64,
    /// Unix-second start of the current daily window.
    pub day_start_secs: u64,
}

/// Result of a quota pre-flight check.
#[derive(Debug, Clone, PartialEq)]
pub enum QuotaResult {
    Ok,
    Exceeded { reason: String },
}

/// Per-user quota enforcement store with sliding time windows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotaStore {
    pub usage: BTreeMap<String, UserUsage>,
    /// Per-user quota overrides that supersede the role default.
    pub overrides: BTreeMap<String, RoleQuota>,
}

impl QuotaStore {
    #[must_use] pub fn new() -> Self { Self::default() }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn get_or_init(&mut self, user_id: &str) -> &mut UserUsage {
        let now = Self::now_secs();
        self.usage.entry(user_id.to_string()).or_insert_with(|| UserUsage {
            user_id: user_id.to_string(),
            window_start_secs: now,
            day_start_secs: now,
            ..Default::default()
        })
    }

    fn reset_if_stale(u: &mut UserUsage, now: u64) {
        if now.saturating_sub(u.window_start_secs) >= 3600 {
            u.tokens_this_hour = 0;
            u.window_start_secs = now;
        }
        if now.saturating_sub(u.day_start_secs) >= 86400 {
            u.cost_usd_today = 0.0;
            u.day_start_secs = now;
        }
    }

    /// Pre-flight check: would adding `tokens`/`cost_usd` and one new run exceed quota?
    pub fn check_quota(&mut self, user_id: &str, role: Role, tokens: u64, cost_usd: f64) -> QuotaResult {
        let quota = self.overrides.get(user_id).cloned()
            .unwrap_or_else(|| default_quota_for_role(role));
        let now = Self::now_secs();
        let u = self.get_or_init(user_id);
        Self::reset_if_stale(u, now);

        if quota.max_tokens_per_hour > 0 && u.tokens_this_hour + tokens > quota.max_tokens_per_hour {
            return QuotaResult::Exceeded {
                reason: format!("token quota: {}/{} tokens this hour", u.tokens_this_hour, quota.max_tokens_per_hour),
            };
        }
        if quota.max_cost_usd_per_day > 0.0 && u.cost_usd_today + cost_usd > quota.max_cost_usd_per_day {
            return QuotaResult::Exceeded {
                reason: format!("cost quota: ${:.4}/${:.4} today", u.cost_usd_today, quota.max_cost_usd_per_day),
            };
        }
        if quota.max_concurrent_runs > 0 && u.active_runs >= quota.max_concurrent_runs {
            return QuotaResult::Exceeded {
                reason: format!("concurrent-run quota: {}/{}", u.active_runs, quota.max_concurrent_runs),
            };
        }
        QuotaResult::Ok
    }

    /// Record consumed tokens and cost after a run completes.
    pub fn record_usage(&mut self, user_id: &str, tokens: u64, cost_usd: f64) {
        let now = Self::now_secs();
        let u = self.get_or_init(user_id);
        Self::reset_if_stale(u, now);
        u.tokens_this_hour += tokens;
        u.cost_usd_today += cost_usd;
    }

    /// Call when an agent run starts.
    pub fn increment_active_runs(&mut self, user_id: &str) {
        let now = Self::now_secs();
        let u = self.get_or_init(user_id);
        Self::reset_if_stale(u, now);
        u.active_runs += 1;
    }

    /// Call when an agent run completes or errors.
    pub fn decrement_active_runs(&mut self, user_id: &str) {
        if let Some(u) = self.usage.get_mut(user_id) {
            u.active_runs = u.active_runs.saturating_sub(1);
        }
    }

    /// Override quotas for a specific user (takes precedence over role default).
    pub fn set_override(&mut self, user_id: &str, quota: RoleQuota) {
        self.overrides.insert(user_id.to_string(), quota);
    }

    #[must_use] pub fn get_usage(&self, user_id: &str) -> Option<&UserUsage> {
        self.usage.get(user_id)
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
            active_team_id: None,
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
            active_team_id: None,
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

    // --- Quota tests ---

    #[test]
    fn quota_viewer_blocked_by_tokens() {
        let mut store = QuotaStore::new();
        // Pre-fill near the limit
        store.record_usage("u1", 9_999, 0.0);
        // Requesting 2 more tokens should exceed the 10k hourly cap
        let res = store.check_quota("u1", Role::Viewer, 2, 0.0);
        assert!(matches!(res, QuotaResult::Exceeded { .. }));
    }

    #[test]
    fn quota_developer_within_limits() {
        let mut store = QuotaStore::new();
        let res = store.check_quota("dev1", Role::Developer, 1_000, 0.01);
        assert_eq!(res, QuotaResult::Ok);
    }

    #[test]
    fn quota_admin_always_ok() {
        let mut store = QuotaStore::new();
        store.record_usage("admin1", 999_999_999, 99999.0);
        let res = store.check_quota("admin1", Role::Admin, 1_000_000, 500.0);
        assert_eq!(res, QuotaResult::Ok);
    }

    #[test]
    fn quota_concurrent_run_limit() {
        let mut store = QuotaStore::new();
        store.increment_active_runs("dev1");
        store.increment_active_runs("dev1");
        store.increment_active_runs("dev1"); // at limit (3 for Developer)
        let res = store.check_quota("dev1", Role::Developer, 0, 0.0);
        assert!(matches!(res, QuotaResult::Exceeded { ref reason } if reason.contains("concurrent")));
        store.decrement_active_runs("dev1");
        let res2 = store.check_quota("dev1", Role::Developer, 0, 0.0);
        assert_eq!(res2, QuotaResult::Ok);
    }

    #[test]
    fn quota_per_user_override() {
        let mut store = QuotaStore::new();
        store.set_override("power-dev", RoleQuota {
            max_tokens_per_hour: 1_000_000,
            max_cost_usd_per_day: 50.0,
            max_concurrent_runs: 20,
        });
        store.record_usage("power-dev", 500_000, 0.0);
        let res = store.check_quota("power-dev", Role::Developer, 400_000, 0.0);
        assert_eq!(res, QuotaResult::Ok); // override, not the 100k role default
    }

    #[test]
    fn quota_cost_limit_exceeded() {
        let mut store = QuotaStore::new();
        store.record_usage("dev2", 0, 4.99);
        let res = store.check_quota("dev2", Role::Developer, 0, 0.02);
        assert!(matches!(res, QuotaResult::Exceeded { ref reason } if reason.contains("cost")));
    }
}
