//! Team Workspace management for the Tachy platform.
//! Manages team creation, membership, invitations, and shared resources.

use audit::Role;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A team workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub members: BTreeMap<String, TeamMember>,
}

/// A member of a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub user_id: String,
    pub role: Role,
    pub joined_at: u64,
}

/// An invitation to join a team workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInvitation {
    pub token: String,
    pub team_id: String,
    pub email: String,
    pub role: Role,
    pub created_at: u64,
    /// Expiry time: `created_at` + 72h (259200 seconds).
    pub expires_at: u64,
    pub used: bool,
}

/// Errors from team operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamError {
    TeamNotFound,
    NotAdmin,
    LastAdmin,
    InvitationExpired,
    InvitationUsed,
    InvitationNotFound,
    AlreadyMember,
    MemberNotFound,
}

impl std::fmt::Display for TeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TeamNotFound => write!(f, "team not found"),
            Self::NotAdmin => write!(f, "only admins can perform this action"),
            Self::LastAdmin => write!(f, "team must have at least one admin"),
            Self::InvitationExpired => write!(f, "invitation expired"),
            Self::InvitationUsed => write!(f, "invitation already used"),
            Self::InvitationNotFound => write!(f, "invitation not found"),
            Self::AlreadyMember => write!(f, "user is already a team member"),
            Self::MemberNotFound => write!(f, "member not found"),
        }
    }
}

/// 72 hours in seconds.
const INVITATION_EXPIRY_SECS: u64 = 259_200;

/// Manages teams and workspace invitations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamManager {
    teams: BTreeMap<String, Team>,
    invitations: BTreeMap<String, WorkspaceInvitation>,
    #[serde(default)]
    team_counter: u64,
    #[serde(default)]
    invite_counter: u64,
}

impl TeamManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a reference to the teams map.
    #[must_use] pub fn teams(&self) -> &BTreeMap<String, Team> {
        &self.teams
    }

    /// Returns a reference to the invitations map.
    #[must_use] pub fn invitations(&self) -> &BTreeMap<String, WorkspaceInvitation> {
        &self.invitations
    }

    /// Look up a user's role in a specific team.
    #[must_use] pub fn get_member_role(&self, team_id: &str, user_id: &str) -> Option<Role> {
        self.teams
            .get(team_id)
            .and_then(|t| t.members.get(user_id))
            .map(|m| m.role)
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Create a new team. The creator becomes the sole Admin member.
    pub fn create_team(&mut self, name: &str, admin_user_id: &str) -> Result<String, TeamError> {
        self.team_counter += 1;
        let id = format!("team-{}", self.team_counter);
        let now = Self::now();

        let mut members = BTreeMap::new();
        members.insert(
            admin_user_id.to_string(),
            TeamMember {
                user_id: admin_user_id.to_string(),
                role: Role::Admin,
                joined_at: now,
            },
        );

        let team = Team {
            id: id.clone(),
            name: name.to_string(),
            created_at: now,
            members,
        };
        self.teams.insert(id.clone(), team);
        Ok(id)
    }

    /// Invite a user to a team. Generates a token with 72h expiry.
    /// Only an Admin of the team can invite.
    pub fn invite(
        &mut self,
        team_id: &str,
        email: &str,
        role: Role,
        inviter_id: &str,
    ) -> Result<String, TeamError> {
        let team = self.teams.get(team_id).ok_or(TeamError::TeamNotFound)?;

        // Verify inviter is an Admin
        let inviter = team.members.get(inviter_id).ok_or(TeamError::NotAdmin)?;
        if inviter.role != Role::Admin {
            return Err(TeamError::NotAdmin);
        }

        self.invite_counter += 1;
        let token = format!("inv-{}", self.invite_counter);
        let now = Self::now();

        let invitation = WorkspaceInvitation {
            token: token.clone(),
            team_id: team_id.to_string(),
            email: email.to_string(),
            role,
            created_at: now,
            expires_at: now + INVITATION_EXPIRY_SECS,
            used: false,
        };
        self.invitations.insert(token.clone(), invitation);
        Ok(token)
    }

    /// Join a team using an invitation token.
    /// Validates the token is not expired and not already used.
    pub fn join(&mut self, token: &str, user_id: &str) -> Result<TeamMember, TeamError> {
        self.join_at(token, user_id, Self::now())
    }

    /// Join a team at a specific timestamp (for testing).
    pub fn join_at(&mut self, token: &str, user_id: &str, now: u64) -> Result<TeamMember, TeamError> {
        let invitation = self
            .invitations
            .get(token)
            .ok_or(TeamError::InvitationNotFound)?;

        if invitation.used {
            return Err(TeamError::InvitationUsed);
        }
        if now >= invitation.expires_at {
            return Err(TeamError::InvitationExpired);
        }

        let team_id = invitation.team_id.clone();
        let role = invitation.role;

        let team = self.teams.get(&team_id).ok_or(TeamError::TeamNotFound)?;
        if team.members.contains_key(user_id) {
            return Err(TeamError::AlreadyMember);
        }

        // Mark invitation as used
        self.invitations.get_mut(token).unwrap().used = true;

        let member = TeamMember {
            user_id: user_id.to_string(),
            role,
            joined_at: now,
        };

        self.teams
            .get_mut(&team_id)
            .unwrap()
            .members
            .insert(user_id.to_string(), member.clone());

        Ok(member)
    }

    /// Update a member's role. Only an Admin can change roles.
    /// Rejects demoting the last Admin.
    pub fn update_member_role(
        &mut self,
        team_id: &str,
        user_id: &str,
        new_role: Role,
        admin_id: &str,
    ) -> Result<(), TeamError> {
        let team = self.teams.get(team_id).ok_or(TeamError::TeamNotFound)?;

        // Verify caller is Admin
        let caller = team.members.get(admin_id).ok_or(TeamError::NotAdmin)?;
        if caller.role != Role::Admin {
            return Err(TeamError::NotAdmin);
        }

        let target = team.members.get(user_id).ok_or(TeamError::MemberNotFound)?;

        // If demoting an Admin, check last-admin invariant
        if target.role == Role::Admin && new_role != Role::Admin {
            let admin_count = team.members.values().filter(|m| m.role == Role::Admin).count();
            if admin_count <= 1 {
                return Err(TeamError::LastAdmin);
            }
        }

        self.teams
            .get_mut(team_id)
            .unwrap()
            .members
            .get_mut(user_id)
            .unwrap()
            .role = new_role;

        Ok(())
    }

    /// Remove a member from a team. Only an Admin can remove members.
    /// Rejects removing the last Admin.
    pub fn remove_member(
        &mut self,
        team_id: &str,
        user_id: &str,
        admin_id: &str,
    ) -> Result<(), TeamError> {
        let team = self.teams.get(team_id).ok_or(TeamError::TeamNotFound)?;

        // Verify caller is Admin
        let caller = team.members.get(admin_id).ok_or(TeamError::NotAdmin)?;
        if caller.role != Role::Admin {
            return Err(TeamError::NotAdmin);
        }

        let target = team.members.get(user_id).ok_or(TeamError::MemberNotFound)?;

        // If removing an Admin, check last-admin invariant
        if target.role == Role::Admin {
            let admin_count = team.members.values().filter(|m| m.role == Role::Admin).count();
            if admin_count <= 1 {
                return Err(TeamError::LastAdmin);
            }
        }

        self.teams.get_mut(team_id).unwrap().members.remove(user_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_team_sets_creator_as_admin() {
        let mut mgr = TeamManager::new();
        let id = mgr.create_team("Engineering", "user-1").unwrap();
        let team = &mgr.teams()[&id];
        assert_eq!(team.name, "Engineering");
        assert_eq!(team.members.len(), 1);
        let member = &team.members["user-1"];
        assert_eq!(member.role, Role::Admin);
    }

    #[test]
    fn invite_and_join_lifecycle() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team A", "admin-1").unwrap();

        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();

        let member = mgr.join(&token, "user-2").unwrap();
        assert_eq!(member.role, Role::Developer);
        assert_eq!(member.user_id, "user-2");

        // Verify user is in the team
        let team = &mgr.teams()[&team_id];
        assert_eq!(team.members.len(), 2);
        assert!(team.members.contains_key("user-2"));
    }

    #[test]
    fn expired_invitation_rejected() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team B", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Viewer, "admin-1")
            .unwrap();

        // Try to join far in the future (past 72h expiry)
        let far_future = mgr.invitations()[&token].expires_at + 1;
        let err = mgr.join_at(&token, "user-3", far_future).unwrap_err();
        assert_eq!(err, TeamError::InvitationExpired);
    }

    #[test]
    fn used_invitation_rejected() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team C", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();

        mgr.join(&token, "user-2").unwrap();

        // Second join with same token should fail
        let err = mgr.join(&token, "user-3").unwrap_err();
        assert_eq!(err, TeamError::InvitationUsed);
    }

    #[test]
    fn last_admin_protection_on_remove() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team D", "admin-1").unwrap();

        // Cannot remove the only admin
        let err = mgr.remove_member(&team_id, "admin-1", "admin-1").unwrap_err();
        assert_eq!(err, TeamError::LastAdmin);
    }

    #[test]
    fn last_admin_protection_on_demote() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team E", "admin-1").unwrap();

        // Cannot demote the only admin
        let err = mgr
            .update_member_role(&team_id, "admin-1", Role::Developer, "admin-1")
            .unwrap_err();
        assert_eq!(err, TeamError::LastAdmin);
    }

    #[test]
    fn update_member_role_success() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team F", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();

        // Promote user-2 to Admin
        mgr.update_member_role(&team_id, "user-2", Role::Admin, "admin-1")
            .unwrap();
        assert_eq!(mgr.teams()[&team_id].members["user-2"].role, Role::Admin);

        // Now we can demote admin-1 since user-2 is also Admin
        mgr.update_member_role(&team_id, "admin-1", Role::Viewer, "user-2")
            .unwrap();
        assert_eq!(mgr.teams()[&team_id].members["admin-1"].role, Role::Viewer);
    }

    #[test]
    fn remove_member_success() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team G", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();
        assert_eq!(mgr.teams()[&team_id].members.len(), 2);

        mgr.remove_member(&team_id, "user-2", "admin-1").unwrap();
        assert_eq!(mgr.teams()[&team_id].members.len(), 1);
        assert!(!mgr.teams()[&team_id].members.contains_key("user-2"));
    }

    #[test]
    fn non_admin_cannot_invite() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team H", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();

        // user-2 (Developer) cannot invite
        let err = mgr
            .invite(&team_id, "other@example.com", Role::Viewer, "user-2")
            .unwrap_err();
        assert_eq!(err, TeamError::NotAdmin);
    }

    #[test]
    fn non_admin_cannot_update_role() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team I", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();

        let err = mgr
            .update_member_role(&team_id, "admin-1", Role::Viewer, "user-2")
            .unwrap_err();
        assert_eq!(err, TeamError::NotAdmin);
    }

    #[test]
    fn non_admin_cannot_remove() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Team J", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();

        let err = mgr
            .remove_member(&team_id, "admin-1", "user-2")
            .unwrap_err();
        assert_eq!(err, TeamError::NotAdmin);
    }

    #[test]
    fn serde_round_trip() {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("Serialized", "admin-1").unwrap();
        let token = mgr
            .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
            .unwrap();
        mgr.join(&token, "user-2").unwrap();

        let json = serde_json::to_string(&mgr).unwrap();
        let restored: TeamManager = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.teams().len(), 1);
        let team = &restored.teams()[&team_id];
        assert_eq!(team.name, "Serialized");
        assert_eq!(team.members.len(), 2);
        assert_eq!(team.members["admin-1"].role, Role::Admin);
        assert_eq!(team.members["user-2"].role, Role::Developer);
    }
}
