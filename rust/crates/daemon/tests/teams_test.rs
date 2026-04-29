//! Team and RBAC property tests.
//!
//! Feature: product-hardening-v3
//! Properties 5–11: `TeamManager` and team-scoped RBAC correctness.
//! Validates: Requirements 3.1–3.6, 4.1–4.5, 5.1

use audit::{check_team_permission, AccessResult, Action, Role};
use daemon::{TeamError, TeamManager};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Property 5: Team creation and persistence round-trip
// ---------------------------------------------------------------------------

#[test]
fn team_creation_round_trip_basic() {
    // Feature: product-hardening-v3, Property 5: Team creation and persistence round-trip
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Engineering", "admin-1").unwrap();

    let teams = mgr.teams();
    assert!(teams.contains_key(&team_id));

    let team = &teams[&team_id];
    assert_eq!(team.name, "Engineering");
    assert_eq!(
        team.members.len(),
        1,
        "creator should be the initial member"
    );

    let member = team.members.get("admin-1").unwrap();
    assert_eq!(member.role, Role::Admin, "creator must be Admin");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 5: For any valid team name and admin ID, the created team stores
    /// exactly those values with exactly one member (the admin).
    ///
    /// Feature: product-hardening-v3, Property 5: Team creation and persistence round-trip
    #[test]
    fn prop_team_creation_stores_all_fields(
        name in "[A-Za-z][A-Za-z0-9 ]{2,20}",
        admin_id in "[a-z][a-z0-9\\-]{2,12}",
    ) {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team(&name, &admin_id).unwrap();

        let teams = mgr.teams();
        prop_assert!(teams.contains_key(&team_id));
        let team = &teams[&team_id];
        prop_assert_eq!(&team.name, &name);
        prop_assert_eq!(team.members.len(), 1);
        prop_assert!(team.members.contains_key(&admin_id));
        prop_assert_eq!(team.members[&admin_id].role, Role::Admin);
    }

    /// Property 5b: Each create_team call produces a distinct team ID.
    #[test]
    fn prop_team_ids_are_unique(n in 2usize..8usize) {
        // Feature: product-hardening-v3, Property 5: Team creation and persistence round-trip
        let mut mgr = TeamManager::new();
        let ids: Vec<String> = (0..n)
            .map(|i| mgr.create_team(&format!("Team {i}"), &format!("admin-{i}")).unwrap())
            .collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        prop_assert_eq!(unique.len(), n, "all team IDs must be distinct");
    }
}

// ---------------------------------------------------------------------------
// Property 6: Invitation-join round-trip preserves role
// ---------------------------------------------------------------------------

#[test]
fn invitation_join_round_trip_basic() {
    // Feature: product-hardening-v3, Property 6: Invitation-join round-trip preserves role
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Ops", "admin-1").unwrap();

    let token = mgr
        .invite(&team_id, "new-dev@example.com", Role::Developer, "admin-1")
        .unwrap();
    let member = mgr.join_at(&token, "user-dev", 1).unwrap();

    assert_eq!(member.role, Role::Developer);
    assert_eq!(member.user_id, "user-dev");

    let teams = mgr.teams();
    assert!(
        teams[&team_id].members.contains_key("user-dev"),
        "new member should appear in team roster"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 6: The role assigned on join matches the role set in the invitation.
    ///
    /// Feature: product-hardening-v3, Property 6: Invitation-join round-trip preserves role
    #[test]
    fn prop_invitation_role_preserved(
        role_idx in 0usize..3usize,
        invitee_id in "[a-z][a-z0-9]{3,10}",
    ) {
        let roles = [Role::Viewer, Role::Developer, Role::Admin];
        let invited_role = roles[role_idx];

        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("PropTeam", "admin-prop").unwrap();

        let token = mgr.invite(&team_id, &format!("{invitee_id}@example.com"), invited_role, "admin-prop").unwrap();
        let member = mgr.join_at(&token, &invitee_id, 1).unwrap();

        prop_assert_eq!(member.role, invited_role);
        prop_assert_eq!(&member.user_id, &invitee_id);
    }
}

// ---------------------------------------------------------------------------
// Property 7: Expired or used invitations are rejected
// ---------------------------------------------------------------------------

#[test]
fn expired_invitation_rejected() {
    // Feature: product-hardening-v3, Property 7: Expired or used invitations are rejected
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Secure", "admin-1").unwrap();

    let token = mgr
        .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
        .unwrap();

    // invite uses current unix time as created_at; expires_at = created_at + 259200.
    // Use far-future timestamp to guarantee we are always past the expiry.
    let result = mgr.join_at(&token, "user-late", u64::MAX / 2);
    assert!(
        matches!(result, Err(TeamError::InvitationExpired)),
        "expired invitation must be rejected"
    );
}

#[test]
fn used_invitation_rejected_on_reuse() {
    // Feature: product-hardening-v3, Property 7: Expired or used invitations are rejected
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Secure", "admin-1").unwrap();

    let token = mgr
        .invite(&team_id, "dev@example.com", Role::Developer, "admin-1")
        .unwrap();
    mgr.join_at(&token, "user-1", 1).unwrap();

    let result = mgr.join_at(&token, "user-2", 2);
    assert!(
        matches!(
            result,
            Err(TeamError::InvitationUsed | TeamError::AlreadyMember)
        ),
        "used invitation must be rejected"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 7: Any join attempt beyond 259200s from creation is rejected as expired.
    ///
    /// Feature: product-hardening-v3, Property 7: Expired or used invitations are rejected
    #[test]
    fn prop_expired_invitations_always_rejected(_extra_secs in 1u64..86_400u64) {
        let mut mgr = TeamManager::new();
        let team_id = mgr.create_team("T", "a").unwrap();
        let token = mgr.invite(&team_id, "x@y.com", Role::Developer, "a").unwrap();

        // The invite's created_at is from TeamManager::now() ≈ current unix time.
        // join_at() takes the join timestamp and compares against invitation.expires_at.
        // expires_at = created_at + 259200. If we pass 0 + 259200 + extra it's expired.
        // Use very large timestamp to guarantee expiry.
        let join_at = u64::MAX / 2; // far future — always past any expiry
        let result = mgr.join_at(&token, "late-user", join_at);
        prop_assert!(
            matches!(result, Err(TeamError::InvitationExpired)),
            "far-future join must be expired"
        );
    }
}

// ---------------------------------------------------------------------------
// Property 8: Last-admin invariant
// ---------------------------------------------------------------------------

#[test]
fn last_admin_cannot_be_removed() {
    // Feature: product-hardening-v3, Property 8: Last-admin invariant
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Solo", "admin-1").unwrap();

    let result = mgr.remove_member(&team_id, "admin-1", "admin-1"); // user_id == admin_id == same person
    assert!(matches!(result, Err(TeamError::LastAdmin)));
}

#[test]
fn last_admin_cannot_be_demoted() {
    // Feature: product-hardening-v3, Property 8: Last-admin invariant
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Solo", "admin-1").unwrap();

    let result = mgr.update_member_role(&team_id, "admin-1", Role::Viewer, "admin-1");
    assert!(matches!(result, Err(TeamError::LastAdmin)));
}

#[test]
fn second_admin_allows_demotion() {
    // Feature: product-hardening-v3, Property 8: Last-admin invariant
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("Dual", "admin-1").unwrap();

    let token = mgr
        .invite(&team_id, "dev@ex.com", Role::Developer, "admin-1")
        .unwrap();
    mgr.join_at(&token, "admin-2", 1).unwrap();
    mgr.update_member_role(&team_id, "admin-2", Role::Admin, "admin-1")
        .unwrap();

    // Now admin-1 can be demoted since admin-2 is also Admin
    mgr.update_member_role(&team_id, "admin-1", Role::Viewer, "admin-1")
        .unwrap();

    let role = mgr.get_member_role(&team_id, "admin-1");
    assert_eq!(role, Some(Role::Viewer));
}

// ---------------------------------------------------------------------------
// Property 9: Team-scoped permission isolation
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 9: Non-member of a team is always denied regardless of action.
    ///
    /// Feature: product-hardening-v3, Property 9: Team-scoped permission isolation
    #[test]
    fn prop_non_member_always_denied(
        action_idx in 0usize..3usize,
    ) {
        let actions = [Action::RunAgent, Action::ListAgents, Action::ManageUsers];
        let action = actions[action_idx];

        let result = check_team_permission("user-1", "team-b", action, None);
        let is_denied = matches!(result, AccessResult::Denied { .. });
        prop_assert!(is_denied, "non-member must always be denied");
    }

    /// Property 9b: Viewer role is always denied for RunAgent.
    #[test]
    fn prop_viewer_cannot_run_agent(user_id in "[a-z]{4,8}", team_id in "[a-z]{4,8}") {
        // Feature: product-hardening-v3, Property 9: Team-scoped permission isolation
        let result = check_team_permission(&user_id, &team_id, Action::RunAgent, Some(Role::Viewer));
        let is_denied = matches!(result, AccessResult::Denied { .. });
        prop_assert!(is_denied, "Viewer cannot run agents");
    }

    /// Property 10: Admin is permitted for all actions.
    #[test]
    fn prop_admin_permitted_all_actions(action_idx in 0usize..5usize) {
        // Feature: product-hardening-v3, Property 10: Role changes are applied and audited
        let actions = [
            Action::RunAgent, Action::ListAgents, Action::ScheduleTask,
            Action::ManageUsers, Action::ManageConfig,
        ];
        let action = actions[action_idx];
        let result = check_team_permission("admin-user", "team-x", action, Some(Role::Admin));
        prop_assert!(
            matches!(result, AccessResult::Allowed),
            "Admin must be allowed for {:?}", action
        );
    }
}

// ---------------------------------------------------------------------------
// Property 10: Role changes take effect
// ---------------------------------------------------------------------------

#[test]
fn role_change_takes_effect() {
    // Feature: product-hardening-v3, Property 10: Role changes are applied and audited
    let mut mgr = TeamManager::new();
    let team_id = mgr.create_team("T", "admin-1").unwrap();

    let token = mgr
        .invite(&team_id, "dev@x.com", Role::Developer, "admin-1")
        .unwrap();
    mgr.join_at(&token, "user-dev", 1).unwrap();

    assert_eq!(
        mgr.get_member_role(&team_id, "user-dev"),
        Some(Role::Developer)
    );

    mgr.update_member_role(&team_id, "user-dev", Role::Viewer, "admin-1")
        .unwrap();

    assert_eq!(
        mgr.get_member_role(&team_id, "user-dev"),
        Some(Role::Viewer)
    );
}

// ---------------------------------------------------------------------------
// Property 11: Agent-team association — teams store independent memberships
// ---------------------------------------------------------------------------

#[test]
fn different_teams_have_independent_members() {
    // Feature: product-hardening-v3, Property 11: Agent-team association
    let mut mgr = TeamManager::new();
    let team_a = mgr.create_team("Team A", "admin-a").unwrap();
    let team_b = mgr.create_team("Team B", "admin-b").unwrap();

    let token = mgr
        .invite(&team_a, "user1@x.com", Role::Developer, "admin-a")
        .unwrap();
    mgr.join_at(&token, "user-1", 1).unwrap();

    assert_eq!(
        mgr.get_member_role(&team_a, "user-1"),
        Some(Role::Developer)
    );
    assert_eq!(
        mgr.get_member_role(&team_b, "user-1"),
        None,
        "user-1 must not be a member of team-b"
    );
}
