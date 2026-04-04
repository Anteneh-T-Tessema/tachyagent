//! Model-specific prompt templates. Different model families need different
//! prompting styles for optimal tool use and code generation.

use serde::{Deserialize, Serialize};

/// Prompt template keyed by model family.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub family: ModelFamily,
    /// System prompt prefix — prepended to the agent's system prompt
    pub system_prefix: String,
    /// How to format tool use instructions for this model
    pub tool_instruction_style: ToolInstructionStyle,
    /// Whether this model works better with explicit JSON formatting instructions
    pub needs_json_hint: bool,
    /// Whether to include "think step by step" for better reasoning
    pub chain_of_thought: bool,
    /// Maximum recommended tools per request (small models choke on many tools)
    pub max_tools_per_request: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelFamily {
    Llama,
    Qwen,
    Mistral,
    DeepSeek,
    CodeLlama,
    Gemma,
    Phi,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolInstructionStyle {
    /// Standard: list tools with descriptions
    Standard,
    /// Explicit: add "You MUST use the tool calling format, not print JSON"
    Explicit,
    /// Minimal: only list tool names, no descriptions (for very small models)
    Minimal,
}

/// Detect model family from model name.
pub fn detect_family(model_name: &str) -> ModelFamily {
    let lower = model_name.to_lowercase();
    if lower.contains("qwen") { return ModelFamily::Qwen; }
    if lower.contains("deepseek") { return ModelFamily::DeepSeek; }
    if lower.contains("mistral") || lower.contains("codestral") { return ModelFamily::Mistral; }
    if lower.contains("codellama") { return ModelFamily::CodeLlama; }
    if lower.contains("gemma") { return ModelFamily::Gemma; }
    if lower.contains("phi") { return ModelFamily::Phi; }
    if lower.contains("llama") { return ModelFamily::Llama; }
    ModelFamily::Generic
}

/// Get the optimal prompt template for a model.
pub fn template_for_model(model_name: &str) -> PromptTemplate {
    let family = detect_family(model_name);
    match family {
        ModelFamily::Qwen => PromptTemplate {
            family,
            system_prefix: "You are a precise coding assistant. Follow instructions exactly. When using tools, use the tool calling format — never print raw JSON as text.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Standard,
            needs_json_hint: false,
            chain_of_thought: true,
            max_tools_per_request: 6,
        },
        ModelFamily::Llama => PromptTemplate {
            family,
            system_prefix: "You are a helpful coding assistant. When you need to use a tool, use the function calling format provided. Do not print JSON tool calls as text — use the actual tool calling mechanism.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Explicit,
            needs_json_hint: true,
            chain_of_thought: true,
            max_tools_per_request: 4,
        },
        ModelFamily::Mistral => PromptTemplate {
            family,
            system_prefix: "You are an expert programmer. Use tools when needed to read files, run commands, or search code.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Standard,
            needs_json_hint: false,
            chain_of_thought: false,
            max_tools_per_request: 6,
        },
        ModelFamily::DeepSeek => PromptTemplate {
            family,
            system_prefix: "You are a coding assistant. Answer questions directly from knowledge. Only use tools when the user asks about specific files or wants to run commands.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Minimal,
            needs_json_hint: true,
            chain_of_thought: true,
            max_tools_per_request: 3,
        },
        ModelFamily::CodeLlama => PromptTemplate {
            family,
            system_prefix: "You are a code generation assistant.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Minimal,
            needs_json_hint: true,
            chain_of_thought: false,
            max_tools_per_request: 3,
        },
        ModelFamily::Gemma => PromptTemplate {
            family,
            system_prefix: "You are an expert coding agent. You have access to tools for reading files, writing files, running shell commands, and searching code. Use tools proactively to gather information before answering. Be precise and thorough.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Standard,
            needs_json_hint: false,
            chain_of_thought: true,
            // Gemma 4 handles native function calling well — full tool set
            max_tools_per_request: 6,
        },
        ModelFamily::Phi => PromptTemplate {
            family,
            system_prefix: "You are a helpful assistant.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Explicit,
            needs_json_hint: true,
            chain_of_thought: true,
            max_tools_per_request: 3,
        },
        ModelFamily::Generic => PromptTemplate {
            family,
            system_prefix: "You are a helpful coding assistant with access to tools for reading files, running commands, and searching code.".to_string(),
            tool_instruction_style: ToolInstructionStyle::Standard,
            needs_json_hint: false,
            chain_of_thought: true,
            max_tools_per_request: 6,
        },
    }
}

/// Build an optimized system prompt for a specific model.
pub fn build_optimized_prompt(
    model_name: &str,
    base_prompt: &str,
    context_injection: Option<&str>,
) -> Vec<String> {
    let template = template_for_model(model_name);
    let mut sections = Vec::new();

    // Model-specific prefix
    sections.push(template.system_prefix.clone());

    // Chain of thought instruction
    if template.chain_of_thought {
        sections.push("Think step by step before acting. Explain your reasoning briefly.".to_string());
    }

    // JSON hint for models that print tool calls as text
    if template.needs_json_hint {
        sections.push("IMPORTANT: When using tools, use the function calling mechanism. Do NOT print JSON tool calls as text in your response.".to_string());
    }

    // Base agent prompt
    sections.push(base_prompt.to_string());

    // Context injection
    if let Some(context) = context_injection {
        sections.push(context.to_string());
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_model_families() {
        assert_eq!(detect_family("qwen3-coder:30b"), ModelFamily::Qwen);
        assert_eq!(detect_family("llama3.1:8b"), ModelFamily::Llama);
        assert_eq!(detect_family("mistral:7b"), ModelFamily::Mistral);
        assert_eq!(detect_family("deepseek-coder:latest"), ModelFamily::DeepSeek);
        assert_eq!(detect_family("codellama:7b"), ModelFamily::CodeLlama);
        assert_eq!(detect_family("gemma4:26b"), ModelFamily::Gemma);
        assert_eq!(detect_family("gemma4:31b"), ModelFamily::Gemma);
        assert_eq!(detect_family("gemma4:e4b"), ModelFamily::Gemma);
        assert_eq!(detect_family("gpt-4o"), ModelFamily::Generic);
    }

    #[test]
    fn templates_have_valid_tool_limits() {
        for name in ["qwen3:8b", "llama3.1:8b", "mistral:7b", "deepseek-coder:latest", "codellama:7b"] {
            let template = template_for_model(name);
            assert!(template.max_tools_per_request >= 3);
            assert!(template.max_tools_per_request <= 6);
        }
    }

    #[test]
    fn optimized_prompt_includes_all_sections() {
        let sections = build_optimized_prompt(
            "llama3.1:8b",
            "You are Tachy.",
            Some("# Codebase: 42 files"),
        );
        assert!(sections.len() >= 3);
        assert!(sections.iter().any(|s| s.contains("Tachy")));
        assert!(sections.iter().any(|s| s.contains("42 files")));
        // Llama needs JSON hint
        assert!(sections.iter().any(|s| s.contains("function calling mechanism")));
    }

    #[test]
    fn qwen_does_not_need_json_hint() {
        let sections = build_optimized_prompt("qwen3-coder:30b", "base", None);
        assert!(!sections.iter().any(|s| s.contains("function calling mechanism")));
    }

    #[test]
    fn gemma4_gets_full_tool_support() {
        let template = template_for_model("gemma4:26b");
        assert_eq!(template.family, ModelFamily::Gemma);
        assert_eq!(template.max_tools_per_request, 6);
        assert!(!template.needs_json_hint); // Gemma 4 has native function calling
        assert!(template.chain_of_thought);
        assert_eq!(template.tool_instruction_style, ToolInstructionStyle::Standard);
    }
}
