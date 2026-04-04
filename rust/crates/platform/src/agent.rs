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
        ];
        for t in &templates {
            assert!(!t.name.is_empty());
            assert!(!t.allowed_tools.is_empty());
            assert!(t.max_iterations > 0);
        }
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
}
