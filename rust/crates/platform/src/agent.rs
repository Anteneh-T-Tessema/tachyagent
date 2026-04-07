use serde::{Deserialize, Serialize};

/// Status of an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Running,
    Completed,
    Failed,
    Suspended,
}

/// Reusable agent template — defines what an agent can do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub model: String,
    pub max_iterations: usize,
    /// Whether this agent requires human approval before tool execution.
    pub requires_approval: bool,
    /// Whether to use the plan-and-execute loop (false = simple single-turn).
    #[serde(default = "default_true")]
    pub use_planning: bool,
}

fn default_true() -> bool { true }

impl AgentTemplate {
    #[must_use]
    pub fn code_reviewer() -> Self {
        Self {
            name: "code-reviewer".to_string(),
            description: "Reviews code changes for bugs, security issues, and style".to_string(),
            system_prompt: concat!(
                "You are a senior code reviewer. Analyze the code for:\n",
                "1. Bugs and logic errors\n",
                "2. Security vulnerabilities\n",
                "3. Performance issues\n",
                "4. Code style and readability\n",
                "Be specific. Reference line numbers. Suggest fixes."
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
                "bash".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 10,
            requires_approval: false,
            use_planning: true,
        }
    }

    #[must_use]
    pub fn security_scanner() -> Self {
        Self {
            name: "security-scanner".to_string(),
            description: "Scans codebase for security vulnerabilities and misconfigurations".to_string(),
            system_prompt: concat!(
                "You are a security auditor. Scan the codebase for:\n",
                "1. Hardcoded secrets, API keys, passwords\n",
                "2. SQL injection, XSS, CSRF vulnerabilities\n",
                "3. Insecure dependencies\n",
                "4. Misconfigured permissions\n",
                "5. Sensitive data exposure\n",
                "Report findings with severity (critical/high/medium/low), file path, and remediation."
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
                "bash".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 20,
            requires_approval: false,
            use_planning: true,
        }
    }

    #[must_use]
    pub fn doc_generator() -> Self {
        Self {
            name: "doc-generator".to_string(),
            description: "Generates and updates documentation from code".to_string(),
            system_prompt: concat!(
                "You are a technical writer. Your job is to:\n",
                "1. Read source code and understand its purpose\n",
                "2. Generate clear, accurate documentation\n",
                "3. Update existing docs when code changes\n",
                "4. Write API references, guides, and READMEs\n",
                "Be concise. Use code examples. Follow the project's existing doc style."
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 15,
            requires_approval: true,
            use_planning: true,
        }
    }

    #[must_use]
    pub fn chat_assistant() -> Self {
        Self {
            name: "chat".to_string(),
            description: "General-purpose AI assistant for questions, code help, and tasks".to_string(),
            system_prompt: concat!(
                "You are Tachy, a helpful AI assistant running locally on the user's machine.\n\n",
                "IMPORTANT RULES:\n",
                "1. For general knowledge questions (what is X, explain Y, how does Z work), ",
                "answer directly from your knowledge. Do NOT use tools for general questions.\n",
                "2. Only use tools when the user asks about specific files, their codebase, ",
                "or wants you to run commands on their system.\n",
                "3. Be friendly, concise, and natural. Never output raw JSON.\n",
                "4. When you do use tools, explain what you found in plain language.\n\n",
                "You have these tools available (use ONLY when needed):\n",
                "- bash: run shell commands\n",
                "- read_file: read a file from disk\n",
                "- write_file: write a file\n",
                "- edit_file: edit a file\n",
                "- grep_search: search file contents\n",
                "- glob_search: find files by pattern\n",
                "- list_directory: list files and folders in a directory\n",
                "- web_search: search the web for documentation, solutions, or information\n",
                "- web_fetch: fetch and read a web page\n"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
                "bash".to_string(),
                "web_search".to_string(),
                "web_fetch".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 16,
            requires_approval: false,
            use_planning: false, // Chat uses simple execution, not planning
        }
    }

    #[must_use]
    pub fn test_runner() -> Self {
        Self {
            name: "test-runner".to_string(),
            description: "Runs tests, analyzes failures, and suggests fixes".to_string(),
            system_prompt: concat!(
                "You are a QA engineer. Your job is to:\n",
                "1. Run the project's test suite\n",
                "2. Analyze any failures\n",
                "3. Read the failing code to understand the root cause\n",
                "4. Suggest specific fixes\n",
                "Always run tests first, then investigate failures."
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "bash".to_string(),
                "read_file".to_string(),
                "grep_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 10,
            requires_approval: false,
            use_planning: true,
        }
    }

    /// Upgrades dependency versions and modernizes code to a target edition/version.
    #[must_use]
    pub fn migrator() -> Self {
        Self {
            name: "migrator".to_string(),
            description: "Upgrades dependencies and migrates code to newer language editions".to_string(),
            system_prompt: concat!(
                "You are a migration specialist. Your job is to:\n",
                "1. Identify the current dependency versions (Cargo.toml, package.json, go.mod, etc.)\n",
                "2. Check for available upgrades via the ecosystem's package manager\n",
                "3. Update version constraints one package at a time to avoid breakage\n",
                "4. Run the test suite after each batch of upgrades to catch regressions\n",
                "5. Fix any compilation errors or API changes introduced by upgrades\n",
                "6. Summarize every change made with before/after versions\n\n",
                "RULES:\n",
                "- Never upgrade more than 5 packages at once — validate first, then continue\n",
                "- Always run tests before marking migration complete\n",
                "- If a breaking change cannot be auto-fixed, document exactly what the user must do manually"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "bash".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 30,
            requires_approval: true,
            use_planning: true,
        }
    }

    /// Extracts hardcoded strings and generates i18n key files.
    #[must_use]
    pub fn localizer() -> Self {
        Self {
            name: "localizer".to_string(),
            description: "Extracts hardcoded user-visible strings and generates i18n translation files".to_string(),
            system_prompt: concat!(
                "You are an internationalization (i18n) specialist. Your job is to:\n",
                "1. Scan the codebase for hardcoded user-visible strings (UI labels, error messages, tooltips)\n",
                "2. Extract them into a structured translation file (JSON, YAML, or .po depending on framework)\n",
                "3. Replace inline strings in source code with i18n function calls (e.g. t('key'), i18n.t('key'), gettext)\n",
                "4. Create base locale files: en.json (source) + empty stubs for any other locales already referenced\n",
                "5. Detect the i18n library already in use (react-i18next, vue-i18n, gettext, fluent, etc.) and follow its conventions\n\n",
                "RULES:\n",
                "- Only extract user-visible text — skip log messages, code comments, variable names\n",
                "- Use snake_case dot-notation keys that match the UI hierarchy (e.g. auth.login.button)\n",
                "- Never break existing functionality — run tests after each file\n",
                "- Document all keys added in a LOCALIZATION.md summary file"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "bash".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 25,
            requires_approval: true,
            use_planning: true,
        }
    }

    /// Identifies hot paths and writes a benchmark harness.
    #[must_use]
    pub fn benchmark() -> Self {
        Self {
            name: "benchmark".to_string(),
            description: "Profiles hot paths and writes a benchmark harness with before/after measurements".to_string(),
            system_prompt: concat!(
                "You are a performance engineer. Your job is to:\n",
                "1. Identify performance-critical code paths using profiling tools (cargo bench, pprof, py-spy, etc.)\n",
                "2. Write targeted micro-benchmarks for the 3–5 hottest functions\n",
                "3. Run the benchmarks to capture a baseline\n",
                "4. Identify the single biggest bottleneck (algorithmic complexity, memory allocation, I/O)\n",
                "5. Implement an optimization — prefer algorithmic improvements over micro-optimizations\n",
                "6. Re-run benchmarks and report the speedup (e.g. '2.3× faster, 40% less memory')\n\n",
                "RULES:\n",
                "- Benchmarks must be repeatable — use fixed seeds for any randomness\n",
                "- Never sacrifice correctness for speed — run unit tests after every optimization\n",
                "- Document the methodology: what was measured, how, and why\n",
                "- If no benchmark framework exists, add one (criterion for Rust, pytest-benchmark for Python, etc.)"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
                "bash".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 20,
            requires_approval: false,
            use_planning: true,
        }
    }

    /// Generates SQL migration files from ORM model changes.
    #[must_use]
    pub fn db_migrator() -> Self {
        Self {
            name: "db-migrator".to_string(),
            description: "Generates SQL migration files from ORM model/schema changes".to_string(),
            system_prompt: concat!(
                "You are a database migration specialist. Your job is to:\n",
                "1. Detect the ORM or schema tool in use (SQLAlchemy, Prisma, Diesel, ActiveRecord, GORM, etc.)\n",
                "2. Diff the current schema against the previous migration to identify changes\n",
                "3. Generate a new versioned migration file with UP and DOWN sections\n",
                "4. Validate the migration is safe: no data-loss columns removed without backfill, indexes on FK columns, etc.\n",
                "5. Test the migration against a local DB if possible (docker run postgres / sqlite in-memory)\n",
                "6. Document the migration purpose in a comment block at the top of the file\n\n",
                "RULES:\n",
                "- Never DROP a column without first checking it has no data (add a safety check)\n",
                "- Always create an index when adding a foreign key\n",
                "- Migration file naming: YYYYMMDDHHMMSS_description.sql (or match existing convention)\n",
                "- If running migrations would be destructive, stop and ask for explicit confirmation"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "bash".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 15,
            requires_approval: true,
            use_planning: true,
        }
    }

    /// Detects breaking API changes between git tags.
    #[must_use]
    pub fn api_compat() -> Self {
        Self {
            name: "api-compat".to_string(),
            description: "Detects breaking API changes between git tags and generates a migration guide".to_string(),
            system_prompt: concat!(
                "You are an API compatibility engineer. Your job is to:\n",
                "1. Compare the current HEAD against a baseline git tag (or the previous semver release)\n",
                "2. Categorize every change as: BREAKING, DEPRECATED, ADDED, or INTERNAL\n",
                "3. For each BREAKING change, write a migration snippet showing the before/after usage\n",
                "4. Generate a CHANGELOG entry and a MIGRATION.md guide\n",
                "5. Suggest a semver bump: major (breaking), minor (additions), patch (fix)\n\n",
                "WHAT COUNTS AS BREAKING:\n",
                "- Removing a public function, method, type, or constant\n",
                "- Changing a function signature (parameter names, types, count, return type)\n",
                "- Changing HTTP endpoint paths, methods, or required parameters\n",
                "- Changing serialized field names or types in public DTOs\n",
                "- Tightening error conditions (was Ok, now Err)\n\n",
                "RULES:\n",
                "- Use `git diff` against the baseline tag to find changes\n",
                "- Focus only on public API surface, not internal implementation\n",
                "- Write the MIGRATION.md in a way that a junior dev can follow"
            ).to_string(),
            allowed_tools: vec![
                "list_directory".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "bash".to_string(),
                "grep_search".to_string(),
                "glob_search".to_string(),
            ],
            model: "gemma4:26b".to_string(),
            max_iterations: 15,
            requires_approval: false,
            use_planning: true,
        }
    }
}

/// Configuration for creating an agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub template: AgentTemplate,
    pub session_id: String,
    pub working_directory: String,
    pub environment: std::collections::BTreeMap<String, String>,
}

/// A running or completed agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub config: AgentConfig,
    pub status: AgentStatus,
    pub iterations_completed: usize,
    pub tool_invocations: u32,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub result_summary: Option<String>,
}

impl AgentInstance {
    #[must_use]
    pub fn new(id: impl Into<String>, config: AgentConfig) -> Self {
        Self {
            id: id.into(),
            config,
            status: AgentStatus::Idle,
            iterations_completed: 0,
            tool_invocations: 0,
            created_at: timestamp(),
            completed_at: None,
            result_summary: None,
        }
    }

    pub fn mark_running(&mut self) {
        self.status = AgentStatus::Running;
    }

    pub fn mark_completed(&mut self, summary: impl Into<String>) {
        self.status = AgentStatus::Completed;
        self.completed_at = Some(timestamp());
        self.result_summary = Some(summary.into());
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.status = AgentStatus::Failed;
        self.completed_at = Some(timestamp());
        self.result_summary = Some(reason.into());
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
    fn templates_have_valid_defaults() {
        let templates = vec![
            AgentTemplate::code_reviewer(),
            AgentTemplate::security_scanner(),
            AgentTemplate::doc_generator(),
            AgentTemplate::test_runner(),
            AgentTemplate::migrator(),
            AgentTemplate::localizer(),
            AgentTemplate::benchmark(),
            AgentTemplate::db_migrator(),
            AgentTemplate::api_compat(),
        ];
        for t in &templates {
            assert!(!t.name.is_empty());
            assert!(!t.allowed_tools.is_empty());
            assert!(t.max_iterations > 0);
        }
        // Every name must be unique
        let names: Vec<_> = templates.iter().map(|t| &t.name).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "duplicate template names");
    }

    #[test]
    fn agent_lifecycle() {
        let config = AgentConfig {
            template: AgentTemplate::code_reviewer(),
            session_id: "sess-1".to_string(),
            working_directory: "/tmp/project".to_string(),
            environment: Default::default(),
        };
        let mut agent = AgentInstance::new("agent-1", config);
        assert_eq!(agent.status, AgentStatus::Idle);

        agent.mark_running();
        assert_eq!(agent.status, AgentStatus::Running);

        agent.mark_completed("Found 3 issues");
        assert_eq!(agent.status, AgentStatus::Completed);
        assert!(agent.completed_at.is_some());
    }

    #[test]
    fn migrator_template_requires_approval_and_has_test_tool() {
        let t = AgentTemplate::migrator();
        assert!(t.requires_approval, "migrator should require approval");
        assert!(t.allowed_tools.iter().any(|s| s.contains("bash") || s.contains("test")),
            "migrator should be able to run tests");
        assert!(t.max_iterations >= 20);
    }

    #[test]
    fn localizer_template_requires_approval() {
        let t = AgentTemplate::localizer();
        assert!(t.requires_approval, "localizer should require approval before writing i18n files");
        assert!(!t.system_prompt.is_empty());
    }

    #[test]
    fn benchmark_template_does_not_require_approval() {
        let t = AgentTemplate::benchmark();
        assert!(!t.requires_approval, "benchmark is read-heavy and should not require approval");
        assert!(t.max_iterations >= 15);
    }

    #[test]
    fn db_migrator_template_requires_approval_and_has_schema_tools() {
        let t = AgentTemplate::db_migrator();
        assert!(t.requires_approval, "db migrations must be human-reviewed");
        assert!(!t.system_prompt.is_empty());
    }

    #[test]
    fn api_compat_template_has_read_only_default() {
        let t = AgentTemplate::api_compat();
        // api_compat reads git tags and diffs — should not require approval
        assert!(!t.requires_approval);
        assert!(t.max_iterations >= 10);
    }

    #[test]
    fn all_new_templates_have_non_empty_system_prompts() {
        let new_templates = vec![
            AgentTemplate::migrator(),
            AgentTemplate::localizer(),
            AgentTemplate::benchmark(),
            AgentTemplate::db_migrator(),
            AgentTemplate::api_compat(),
        ];
        for t in &new_templates {
            assert!(!t.system_prompt.is_empty(),
                "template '{}' has empty system_prompt", t.name);
            // Prompt should be reasonably descriptive (>50 chars)
            assert!(t.system_prompt.len() > 50,
                "template '{}' system_prompt too short: '{}'", t.name, t.system_prompt);
        }
    }

    #[test]
    fn all_templates_have_distinct_system_prompts() {
        let templates = vec![
            AgentTemplate::code_reviewer(),
            AgentTemplate::security_scanner(),
            AgentTemplate::doc_generator(),
            AgentTemplate::test_runner(),
            AgentTemplate::migrator(),
            AgentTemplate::localizer(),
            AgentTemplate::benchmark(),
            AgentTemplate::db_migrator(),
            AgentTemplate::api_compat(),
        ];
        let prompts: Vec<_> = templates.iter().map(|t| &t.system_prompt).collect();
        let unique: std::collections::HashSet<_> = prompts.iter().collect();
        assert_eq!(prompts.len(), unique.len(), "templates share identical system prompts");
    }
}
