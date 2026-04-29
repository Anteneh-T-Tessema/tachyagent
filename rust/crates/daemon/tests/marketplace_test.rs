//! Marketplace property tests.
//!
//! Feature: product-hardening-v3
//! Properties 12–19: Marketplace correctness.
//! Validates: Requirements 6.1–6.5, 7.1–7.5

use daemon::{Marketplace, MarketplaceError};
use platform::AgentTemplate;
use proptest::prelude::*;

fn chat_template(name: &str) -> AgentTemplate {
    let mut t = AgentTemplate::chat_assistant();
    t.name = name.to_string();
    t
}

// ---------------------------------------------------------------------------
// Property 12: Marketplace publish preserves all listing fields
// ---------------------------------------------------------------------------

#[test]
fn publish_preserves_fields() {
    // Feature: product-hardening-v3, Property 12: Marketplace publish preserves all listing fields
    let mut mp = Marketplace::new();
    let tmpl = chat_template("my-agent");
    let id = mp
        .publish(tmpl, "A chat agent", "1.0.0", "author-1")
        .unwrap();

    let listings = mp.listings();
    let listing = listings.get(&id).unwrap();
    assert_eq!(listing.name, "my-agent");
    assert_eq!(listing.description, "A chat agent");
    assert_eq!(listing.author_id, "author-1");
    assert_eq!(listing.default_version, "1.0.0");
    assert_eq!(listing.versions.len(), 1);
    assert_eq!(listing.versions[0].version, "1.0.0");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 12: For any valid (name, description, version, author),
    /// publish stores all fields exactly.
    ///
    /// Feature: product-hardening-v3, Property 12: Marketplace publish preserves all listing fields
    #[test]
    fn prop_publish_preserves_all_fields(
        name in "[a-z][a-z0-9\\-]{2,15}",
        description in "[A-Za-z ]{5,40}",
        major in 0u32..10u32,
        minor in 0u32..20u32,
        patch in 0u32..100u32,
        author in "[a-z]{4,10}",
    ) {
        let version = format!("{major}.{minor}.{patch}");
        let mut mp = Marketplace::new();
        let tmpl = chat_template(&name);

        let id = mp.publish(tmpl, &description, &version, &author).unwrap();
        let listings = mp.listings();
        let listing = listings.get(&id).unwrap();

        prop_assert_eq!(&listing.name, &name);
        prop_assert_eq!(&listing.description, &description);
        prop_assert_eq!(&listing.author_id, &author);
        prop_assert_eq!(&listing.default_version, &version);
    }
}

// ---------------------------------------------------------------------------
// Property 13: Semver validation
// ---------------------------------------------------------------------------

#[test]
fn invalid_semver_rejected() {
    // Feature: product-hardening-v3, Property 13: Semver validation
    let mut mp = Marketplace::new();
    let cases = [
        "1.0",
        "1",
        "v1.0.0",
        "1.0.0-alpha",
        "1.0.0.0",
        "",
        "one.two.three",
    ];
    for bad in &cases {
        let result = mp.publish(chat_template("t"), "d", bad, "a");
        assert!(
            matches!(result, Err(MarketplaceError::InvalidSemver)),
            "version '{bad}' should be invalid"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 13: X.Y.Z with non-negative integers always accepted.
    ///
    /// Feature: product-hardening-v3, Property 13: Semver validation
    #[test]
    fn prop_valid_semver_accepted(
        x in 0u32..100u32,
        y in 0u32..100u32,
        z in 0u32..1000u32,
    ) {
        let mut mp = Marketplace::new();
        let version = format!("{x}.{y}.{z}");
        // Use a unique agent name per test case
        let name = format!("agent-{x}-{y}-{z}");
        let result = mp.publish(chat_template(&name), "desc", &version, "author");
        prop_assert!(result.is_ok(), "valid semver {version} must be accepted");
    }
}

// ---------------------------------------------------------------------------
// Property 14: Version history is append-only with latest as default
// ---------------------------------------------------------------------------

#[test]
fn version_history_append_only() {
    // Feature: product-hardening-v3, Property 14: Version history is append-only with latest as default
    let mut mp = Marketplace::new();
    mp.publish(chat_template("my-bot"), "Bot", "1.0.0", "a")
        .unwrap();
    mp.publish(chat_template("my-bot"), "Bot v2", "1.1.0", "a")
        .unwrap();
    mp.publish(chat_template("my-bot"), "Bot v3", "2.0.0", "a")
        .unwrap();

    let listings = mp.listings();
    // All versions should be in a single listing
    let listing = listings.values().find(|l| l.name == "my-bot").unwrap();
    assert_eq!(
        listing.versions.len(),
        3,
        "all 3 versions should be retained"
    );
    assert_eq!(
        listing.default_version, "2.0.0",
        "latest version should be default"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 14: Publishing N distinct versions for the same agent
    /// results in exactly N versions stored, with the last one as default.
    ///
    /// Feature: product-hardening-v3, Property 14: Version history is append-only with latest as default
    #[test]
    fn prop_version_history_append_only(n in 2usize..6usize) {
        let mut mp = Marketplace::new();
        let mut last_version = String::new();

        for i in 0..n {
            let version = format!("{i}.0.0");
            mp.publish(chat_template("multi-ver"), "Desc", &version, "auth").unwrap();
            last_version = version;
        }

        let listings = mp.listings();
        let listing = listings.values().find(|l| l.name == "multi-ver").unwrap();
        prop_assert_eq!(listing.versions.len(), n);
        prop_assert_eq!(&listing.default_version, &last_version);
    }
}

// ---------------------------------------------------------------------------
// Property 15: Duplicate name+version conflict detection
// ---------------------------------------------------------------------------

#[test]
fn duplicate_version_rejected() {
    // Feature: product-hardening-v3, Property 15: Duplicate name+version conflict detection
    let mut mp = Marketplace::new();
    mp.publish(chat_template("dupe-bot"), "Bot", "1.0.0", "a")
        .unwrap();
    let result = mp.publish(chat_template("dupe-bot"), "Bot again", "1.0.0", "a");
    assert!(
        matches!(result, Err(MarketplaceError::ConflictVersion)),
        "duplicate name+version must be rejected"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 15: Publishing the same (name, version) twice always returns ConflictVersion.
    ///
    /// Feature: product-hardening-v3, Property 15: Duplicate name+version conflict detection
    #[test]
    fn prop_duplicate_version_always_rejected(
        x in 0u32..10u32,
        y in 0u32..10u32,
        z in 0u32..10u32,
    ) {
        let version = format!("{x}.{y}.{z}");
        let mut mp = Marketplace::new();
        mp.publish(chat_template("conflict-bot"), "d", &version, "a").unwrap();
        let result = mp.publish(chat_template("conflict-bot"), "d2", &version, "a");
        prop_assert!(
            matches!(result, Err(MarketplaceError::ConflictVersion)),
            "second publish of {version} must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// Property 16: Marketplace search results are sorted by rating descending
// ---------------------------------------------------------------------------

#[test]
fn search_sorted_by_rating_descending() {
    // Feature: product-hardening-v3, Property 16: Marketplace search results sorted by rating descending
    let mut mp = Marketplace::new();

    let id_a = mp
        .publish(chat_template("agent-a"), "A", "1.0.0", "u1")
        .unwrap();
    let id_b = mp
        .publish(chat_template("agent-b"), "B", "1.0.0", "u2")
        .unwrap();
    let id_c = mp
        .publish(chat_template("agent-c"), "C", "1.0.0", "u3")
        .unwrap();

    mp.rate(&id_a, "user-1", 5).unwrap(); // avg: 5.0
    mp.rate(&id_c, "user-1", 3).unwrap(); // avg: 3.0
                                          // id_b has no ratings: avg 0.0

    let results = mp.search(None, 0, 10);
    assert_eq!(results.len(), 3);
    assert!(results.iter().any(|item| item.id == id_b));
    assert!(
        results[0].average_rating >= results[1].average_rating,
        "first result should have highest rating"
    );
    assert!(
        results[1].average_rating >= results[2].average_rating,
        "results should be sorted descending by rating"
    );
}

// ---------------------------------------------------------------------------
// Property 17: Marketplace install round-trip
// ---------------------------------------------------------------------------

#[test]
fn install_returns_correct_template() {
    // Feature: product-hardening-v3, Property 17: Marketplace install round-trip
    let mut mp = Marketplace::new();
    let mut tmpl = AgentTemplate::code_reviewer();
    tmpl.name = "code-bot".to_string();
    let id = mp
        .publish(tmpl.clone(), "Code reviewer", "1.0.0", "auth")
        .unwrap();

    let result = mp.install(&id, None).unwrap();
    assert_eq!(result.template.name, "code-bot");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 17: Installing a listing always returns a template with the same name
    /// as the published template.
    ///
    /// Feature: product-hardening-v3, Property 17: Marketplace install round-trip
    #[test]
    fn prop_install_returns_correct_name(name in "[a-z][a-z0-9\\-]{2,12}") {
        let mut mp = Marketplace::new();
        let id = mp.publish(chat_template(&name), "d", "1.0.0", "a").unwrap();
        let result = mp.install(&id, None).unwrap();
        prop_assert_eq!(&result.template.name, &name, "installed template name must match");
    }
}

// ---------------------------------------------------------------------------
// Property 18: Rating average correctness with idempotent per-user updates
// ---------------------------------------------------------------------------

#[test]
fn rating_average_correct() {
    // Feature: product-hardening-v3, Property 18: Rating average correctness
    let mut mp = Marketplace::new();
    let id = mp
        .publish(chat_template("rated-bot"), "R", "1.0.0", "a")
        .unwrap();

    mp.rate(&id, "user-1", 5).unwrap();
    mp.rate(&id, "user-2", 3).unwrap();
    mp.rate(&id, "user-3", 4).unwrap();

    let listing = &mp.listings()[&id];
    let expected_avg = (5.0 + 3.0 + 4.0) / 3.0;
    assert!(
        (listing.average_rating - expected_avg).abs() < 0.01,
        "average should be {expected_avg:.2}, got {:.2}",
        listing.average_rating
    );
    assert_eq!(listing.rating_count, 3);
}

#[test]
fn rating_is_per_user_idempotent() {
    // Feature: product-hardening-v3, Property 18: Rating average correctness
    let mut mp = Marketplace::new();
    let id = mp
        .publish(chat_template("idem-bot"), "I", "1.0.0", "a")
        .unwrap();

    mp.rate(&id, "user-1", 2).unwrap();
    mp.rate(&id, "user-1", 5).unwrap(); // update — not a second rating

    let listing = &mp.listings()[&id];
    assert_eq!(
        listing.rating_count, 1,
        "user-1 has only one rating (updated)"
    );
    assert!(
        (listing.average_rating - 5.0).abs() < 0.01,
        "average should reflect the update"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property 18: Average rating always in [1,5] when at least one rating exists.
    #[test]
    fn prop_rating_always_in_range(
        ratings in prop::collection::vec(1u8..=5u8, 1..8usize),
    ) {
        // Feature: product-hardening-v3, Property 18: Rating average correctness
        let mut mp = Marketplace::new();
        let id = mp.publish(chat_template("range-bot"), "d", "1.0.0", "a").unwrap();

        for (i, &r) in ratings.iter().enumerate() {
            mp.rate(&id, &format!("user-{i}"), r).unwrap();
        }

        let listing = &mp.listings()[&id];
        prop_assert!(listing.average_rating >= 1.0 && listing.average_rating <= 5.0,
            "average rating must be in [1,5], got {}", listing.average_rating);
    }
}

// ---------------------------------------------------------------------------
// Property 19: Missing tools detection on install
// ---------------------------------------------------------------------------

#[test]
fn install_with_unknown_tools_warns() {
    // Feature: product-hardening-v3, Property 19: Missing tools detection on install
    let mut mp = Marketplace::new();
    let mut tmpl = chat_template("tool-bot");
    tmpl.allowed_tools = vec!["read_file".to_string(), "nonexistent_tool".to_string()];
    let id = mp.publish(tmpl, "d", "1.0.0", "a").unwrap();

    let known_tools = vec!["read_file".to_string(), "write_file".to_string()];
    let result = mp
        .install_with_tools_check(&id, None, &known_tools)
        .unwrap();

    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("nonexistent_tool")),
        "should warn about the unknown tool"
    );
}

#[test]
fn install_with_all_known_tools_no_warnings() {
    // Feature: product-hardening-v3, Property 19: Missing tools detection on install
    let mut mp = Marketplace::new();
    let mut tmpl = chat_template("clean-bot");
    tmpl.allowed_tools = vec!["read_file".to_string(), "write_file".to_string()];
    let id = mp.publish(tmpl, "d", "1.0.0", "a").unwrap();

    let known_tools = vec!["read_file".to_string(), "write_file".to_string()];
    let result = mp
        .install_with_tools_check(&id, None, &known_tools)
        .unwrap();

    assert!(
        result.warnings.is_empty(),
        "no warnings when all tools are known"
    );
}
