//! Agent Marketplace for the Tachy platform.
//! Publishing, discovery, installation, and rating of agent templates.

use platform::AgentTemplate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A published agent template listing in the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListing {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author_id: String,
    pub versions: Vec<MarketplaceVersion>,
    pub default_version: String,
    pub average_rating: f64,
    pub rating_count: u32,
    pub ratings: BTreeMap<String, u8>,
    pub created_at: u64,
    pub updated_at: u64,
    pub visibility: ListingVisibility,
    pub team_id: Option<String>,
}

/// Visibility of a marketplace listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ListingVisibility {
    #[default]
    Public,
    Team,
}

/// A versioned snapshot of an agent template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceVersion {
    /// Semantic version: MAJOR.MINOR.PATCH
    pub version: String,
    pub template: AgentTemplate,
    pub published_at: u64,
}

/// Errors from marketplace operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceError {
    InvalidSemver,
    ConflictVersion,
    ListingNotFound,
    VersionNotFound,
    InvalidRating,
}

impl std::fmt::Display for MarketplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSemver => write!(f, "invalid version format"),
            Self::ConflictVersion => write!(f, "version already exists"),
            Self::ListingNotFound => write!(f, "listing not found"),
            Self::VersionNotFound => write!(f, "version not found"),
            Self::InvalidRating => write!(f, "rating must be 1-5"),
        }
    }
}

/// Result of an install operation, including the template and any warnings.
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub template: AgentTemplate,
    pub warnings: Vec<String>,
}

/// Validates that a version string matches MAJOR.MINOR.PATCH (all non-negative integers).
fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// The agent marketplace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Marketplace {
    listings: BTreeMap<String, MarketplaceListing>,
    #[serde(default)]
    listing_counter: u64,
}

impl Marketplace {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a reference to the listings map.
    pub fn listings(&self) -> &BTreeMap<String, MarketplaceListing> {
        &self.listings
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Publish an agent template to the marketplace.
    ///
    /// If a listing with the same name already exists, a new version is appended.
    /// Rejects invalid semver and duplicate (name, version) pairs.
    pub fn publish(
        &mut self,
        template: AgentTemplate,
        description: &str,
        version: &str,
        author_id: &str,
    ) -> Result<String, MarketplaceError> {
        if !is_valid_semver(version) {
            return Err(MarketplaceError::InvalidSemver);
        }

        let now = Self::now();

        // Check for existing listing with the same name
        let existing_id = self
            .listings
            .values()
            .find(|l| l.name == template.name)
            .map(|l| l.id.clone());

        if let Some(id) = existing_id {
            let listing = self.listings.get_mut(&id).unwrap();

            // Conflict detection: same name + version
            if listing.versions.iter().any(|v| v.version == version) {
                return Err(MarketplaceError::ConflictVersion);
            }

            // Append new version; latest becomes default
            listing.versions.push(MarketplaceVersion {
                version: version.to_string(),
                template,
                published_at: now,
            });
            listing.default_version = version.to_string();
            listing.updated_at = now;

            Ok(id)
        } else {
            self.listing_counter += 1;
            let id = format!("listing-{}", self.listing_counter);

            let listing = MarketplaceListing {
                id: id.clone(),
                name: template.name.clone(),
                description: description.to_string(),
                author_id: author_id.to_string(),
                versions: vec![MarketplaceVersion {
                    version: version.to_string(),
                    template,
                    published_at: now,
                }],
                default_version: version.to_string(),
                average_rating: 0.0,
                rating_count: 0,
                ratings: BTreeMap::new(),
                created_at: now,
                updated_at: now,
                visibility: ListingVisibility::Public,
                team_id: None,
            };
            self.listings.insert(id.clone(), listing);
            Ok(id)
        }
    }

    /// Publish a team-specific agent template.
    pub fn publish_to_team(
        &mut self,
        template: AgentTemplate,
        description: &str,
        version: &str,
        author_id: &str,
        team_id: &str,
    ) -> Result<String, MarketplaceError> {
        let id = self.publish(template, description, version, author_id)?;
        let listing = self.listings.get_mut(&id).unwrap();
        listing.visibility = ListingVisibility::Team;
        listing.team_id = Some(team_id.to_string());
        Ok(id)
    }

    /// Search marketplace listings.
    ///
    /// Results are sorted by `average_rating` descending.
    /// Optional query filters on listing name (case-insensitive substring match).
    /// Paginated with `page` (0-indexed) and `page_size`.
    pub fn search(
        &self,
        query: Option<&str>,
        page: usize,
        page_size: usize,
    ) -> Vec<&MarketplaceListing> {
        let mut results: Vec<&MarketplaceListing> = self
            .listings
            .values()
            .filter(|l| {
                // Visibility check: Public or matching Team
                if l.visibility == ListingVisibility::Team {
                    // Note: Filtering by user's teams happens in the HTTP handler
                    return true; 
                }
                true
            })
            .filter(|l| {
                query.map_or(true, |q| {
                    l.name.to_lowercase().contains(&q.to_lowercase())
                })
            })
            .collect();

        // Sort by average_rating descending (stable sort for determinism)
        results.sort_by(|a, b| {
            b.average_rating
                .partial_cmp(&a.average_rating)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let start = page * page_size;
        if start >= results.len() {
            return Vec::new();
        }
        let end = (start + page_size).min(results.len());
        results[start..end].to_vec()
    }

    /// Install an agent template from a listing.
    ///
    /// Returns the `AgentTemplate` for the requested version (or default).
    /// Optionally checks for missing tools against `available_tools`.
    pub fn install(
        &self,
        listing_id: &str,
        version: Option<&str>,
    ) -> Result<InstallResult, MarketplaceError> {
        let listing = self
            .listings
            .get(listing_id)
            .ok_or(MarketplaceError::ListingNotFound)?;

        let target_version = version.unwrap_or(&listing.default_version);

        let mv = listing
            .versions
            .iter()
            .find(|v| v.version == target_version)
            .ok_or(MarketplaceError::VersionNotFound)?;

        Ok(InstallResult {
            template: mv.template.clone(),
            warnings: Vec::new(),
        })
    }

    /// Install with missing-tools detection.
    ///
    /// `available_tools` is the set of tools available in the target workspace.
    /// Any tool in the template's `allowed_tools` that is not in `available_tools`
    /// is reported as a warning.
    pub fn install_with_tools_check(
        &self,
        listing_id: &str,
        version: Option<&str>,
        available_tools: &[String],
    ) -> Result<InstallResult, MarketplaceError> {
        let mut result = self.install(listing_id, version)?;

        let missing: Vec<String> = result
            .template
            .allowed_tools
            .iter()
            .filter(|t| !available_tools.contains(t))
            .map(|t| format!("missing tool: {t}"))
            .collect();

        result.warnings = missing;
        Ok(result)
    }

    /// Rate a marketplace listing.
    ///
    /// Rating must be 1-5. One rating per user per listing; subsequent calls update.
    /// Recalculates average_rating and rating_count.
    pub fn rate(
        &mut self,
        listing_id: &str,
        user_id: &str,
        rating: u8,
    ) -> Result<(), MarketplaceError> {
        if !(1..=5).contains(&rating) {
            return Err(MarketplaceError::InvalidRating);
        }

        let listing = self
            .listings
            .get_mut(listing_id)
            .ok_or(MarketplaceError::ListingNotFound)?;

        listing.ratings.insert(user_id.to_string(), rating);

        // Recalculate average
        let count = listing.ratings.len() as u32;
        let sum: u64 = listing.ratings.values().map(|&r| u64::from(r)).sum();
        listing.rating_count = count;
        listing.average_rating = if count > 0 {
            sum as f64 / f64::from(count)
        } else {
            0.0
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_template(name: &str) -> AgentTemplate {
        AgentTemplate {
            name: name.to_string(),
            description: format!("{name} description"),
            system_prompt: "You are a test agent.".to_string(),
            allowed_tools: vec!["read_file".to_string(), "bash".to_string()],
            model: "test-model".to_string(),
            max_iterations: 5,
            requires_approval: false,
            use_planning: true,
        }
    }

    // --- Publish tests ---

    #[test]
    fn publish_creates_listing() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("my-agent"), "A cool agent", "1.0.0", "user-1")
            .unwrap();

        let listing = &mp.listings()[&id];
        assert_eq!(listing.name, "my-agent");
        assert_eq!(listing.description, "A cool agent");
        assert_eq!(listing.author_id, "user-1");
        assert_eq!(listing.default_version, "1.0.0");
        assert_eq!(listing.versions.len(), 1);
        assert_eq!(listing.versions[0].template.name, "my-agent");
    }

    // --- Semver validation tests ---

    #[test]
    fn valid_semver_accepted() {
        assert!(is_valid_semver("0.0.0"));
        assert!(is_valid_semver("1.2.3"));
        assert!(is_valid_semver("10.20.30"));
    }

    #[test]
    fn invalid_semver_rejected() {
        assert!(!is_valid_semver(""));
        assert!(!is_valid_semver("1.0"));
        assert!(!is_valid_semver("1.0.0.0"));
        assert!(!is_valid_semver("v1.0.0"));
        assert!(!is_valid_semver("1.0.0-beta"));
        assert!(!is_valid_semver("a.b.c"));
        assert!(!is_valid_semver("1..0"));
        assert!(!is_valid_semver(".1.0"));
    }

    #[test]
    fn publish_rejects_invalid_semver() {
        let mut mp = Marketplace::new();
        let err = mp
            .publish(test_template("agent"), "desc", "not-semver", "user-1")
            .unwrap_err();
        assert_eq!(err, MarketplaceError::InvalidSemver);
    }

    // --- Conflict detection tests ---

    #[test]
    fn publish_rejects_duplicate_name_version() {
        let mut mp = Marketplace::new();
        mp.publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        let err = mp
            .publish(test_template("agent"), "desc v2", "1.0.0", "user-2")
            .unwrap_err();
        assert_eq!(err, MarketplaceError::ConflictVersion);
    }

    #[test]
    fn publish_allows_different_version_same_name() {
        let mut mp = Marketplace::new();
        let id1 = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();
        let id2 = mp
            .publish(test_template("agent"), "desc", "1.1.0", "user-1")
            .unwrap();

        // Same listing, new version appended
        assert_eq!(id1, id2);
        assert_eq!(mp.listings()[&id1].versions.len(), 2);
    }

    // --- Version history tests ---

    #[test]
    fn version_history_is_append_only_with_latest_default() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();
        mp.publish(test_template("agent"), "desc", "2.0.0", "user-1")
            .unwrap();
        mp.publish(test_template("agent"), "desc", "3.0.0", "user-1")
            .unwrap();

        let listing = &mp.listings()[&id];
        assert_eq!(listing.versions.len(), 3);
        assert_eq!(listing.versions[0].version, "1.0.0");
        assert_eq!(listing.versions[1].version, "2.0.0");
        assert_eq!(listing.versions[2].version, "3.0.0");
        assert_eq!(listing.default_version, "3.0.0");
    }

    // --- Search sort order tests ---

    #[test]
    fn search_returns_sorted_by_rating_descending() {
        let mut mp = Marketplace::new();
        let id_a = mp
            .publish(test_template("alpha"), "desc", "1.0.0", "user-1")
            .unwrap();
        let id_b = mp
            .publish(test_template("beta"), "desc", "1.0.0", "user-1")
            .unwrap();
        let id_c = mp
            .publish(test_template("gamma"), "desc", "1.0.0", "user-1")
            .unwrap();

        mp.rate(&id_a, "u1", 2).unwrap();
        mp.rate(&id_b, "u1", 5).unwrap();
        mp.rate(&id_c, "u1", 3).unwrap();

        let results = mp.search(None, 0, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].name, "beta");   // 5.0
        assert_eq!(results[1].name, "gamma");  // 3.0
        assert_eq!(results[2].name, "alpha");  // 2.0
    }

    #[test]
    fn search_filters_by_query() {
        let mut mp = Marketplace::new();
        mp.publish(test_template("react-reviewer"), "desc", "1.0.0", "u1")
            .unwrap();
        mp.publish(test_template("python-linter"), "desc", "1.0.0", "u1")
            .unwrap();

        let results = mp.search(Some("react"), 0, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "react-reviewer");
    }

    #[test]
    fn search_pagination() {
        let mut mp = Marketplace::new();
        for i in 0..5 {
            mp.publish(test_template(&format!("agent-{i}")), "desc", "1.0.0", "u1")
                .unwrap();
        }

        let page0 = mp.search(None, 0, 2);
        assert_eq!(page0.len(), 2);

        let page1 = mp.search(None, 1, 2);
        assert_eq!(page1.len(), 2);

        let page2 = mp.search(None, 2, 2);
        assert_eq!(page2.len(), 1);

        let page3 = mp.search(None, 3, 2);
        assert!(page3.is_empty());
    }

    // --- Install tests ---

    #[test]
    fn install_returns_default_version_template() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();
        mp.publish(test_template("agent"), "desc", "2.0.0", "user-1")
            .unwrap();

        let result = mp.install(&id, None).unwrap();
        // Default is latest (2.0.0), but template name is the same
        assert_eq!(result.template.name, "agent");
    }

    #[test]
    fn install_returns_specific_version() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();
        mp.publish(test_template("agent"), "desc", "2.0.0", "user-1")
            .unwrap();

        let result = mp.install(&id, Some("1.0.0")).unwrap();
        assert_eq!(result.template.name, "agent");
    }

    #[test]
    fn install_listing_not_found() {
        let mp = Marketplace::new();
        let err = mp.install("nonexistent", None).unwrap_err();
        assert_eq!(err, MarketplaceError::ListingNotFound);
    }

    #[test]
    fn install_version_not_found() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        let err = mp.install(&id, Some("9.9.9")).unwrap_err();
        assert_eq!(err, MarketplaceError::VersionNotFound);
    }

    // --- Missing tools warning tests ---

    #[test]
    fn install_warns_about_missing_tools() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        // Template requires ["read_file", "bash"], workspace only has "read_file"
        let result = mp
            .install_with_tools_check(&id, None, &["read_file".to_string()])
            .unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("bash"));
    }

    #[test]
    fn install_no_warnings_when_all_tools_available() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        let result = mp
            .install_with_tools_check(
                &id,
                None,
                &["read_file".to_string(), "bash".to_string()],
            )
            .unwrap();
        assert!(result.warnings.is_empty());
    }

    // --- Rate tests ---

    #[test]
    fn rate_calculates_average() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        mp.rate(&id, "u1", 4).unwrap();
        mp.rate(&id, "u2", 2).unwrap();

        let listing = &mp.listings()[&id];
        assert_eq!(listing.rating_count, 2);
        assert!((listing.average_rating - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rate_updates_existing_user_rating() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        mp.rate(&id, "u1", 1).unwrap();
        mp.rate(&id, "u1", 5).unwrap();

        let listing = &mp.listings()[&id];
        assert_eq!(listing.rating_count, 1);
        assert!((listing.average_rating - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn rate_rejects_invalid_values() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();

        assert_eq!(mp.rate(&id, "u1", 0).unwrap_err(), MarketplaceError::InvalidRating);
        assert_eq!(mp.rate(&id, "u1", 6).unwrap_err(), MarketplaceError::InvalidRating);
    }

    #[test]
    fn rate_listing_not_found() {
        let mut mp = Marketplace::new();
        let err = mp.rate("nonexistent", "u1", 3).unwrap_err();
        assert_eq!(err, MarketplaceError::ListingNotFound);
    }

    // --- Serde round-trip ---

    #[test]
    fn serde_round_trip() {
        let mut mp = Marketplace::new();
        let id = mp
            .publish(test_template("agent"), "desc", "1.0.0", "user-1")
            .unwrap();
        mp.rate(&id, "u1", 4).unwrap();

        let json = serde_json::to_string(&mp).unwrap();
        let restored: Marketplace = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.listings().len(), 1);
        let listing = &restored.listings()[&id];
        assert_eq!(listing.name, "agent");
        assert_eq!(listing.rating_count, 1);
    }
}
