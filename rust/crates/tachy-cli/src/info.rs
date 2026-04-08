//! Listing commands: models, agents, tools, channels.

use std::env;

use backend::BackendRegistry;
use platform::PlatformConfig;

pub(crate) fn json_output_enabled() -> bool {
    std::env::var("TACHY_OUTPUT_JSON").map(|v| v == "1").unwrap_or(false)
}

pub(crate) fn list_channels() {
    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let channels = daemon::load_channels(&tachy_dir);
    if channels.is_empty() {
        println!("No messaging channels configured.");
        println!("Create .tachy/channels.yaml to add Slack, Discord, or Telegram integration.");
        println!("\nExample:");
        println!("  channels:");
        println!("    - type: slack");
        println!("      bot_token: $SLACK_BOT_TOKEN");
        println!("      channel: \"#ai-agent\"");
        println!("      template: chat");
        return;
    }
    println!("Configured channels:\n");
    for ch in &channels {
        let status = if ch.enabled { "enabled" } else { "disabled" };
        println!("  {:12} {:10} template={:15} [{}]",
            format!("{:?}", ch.r#type).to_lowercase(),
            if ch.channel.is_empty() { &ch.channel_id } else { &ch.channel },
            if ch.template.is_empty() { "chat" } else { &ch.template },
            status,
        );
    }
}

pub(crate) fn list_tools() {
    println!("Built-in tools:\n");
    for spec in tools::mvp_tool_specs() {
        println!("  {:20} {}", spec.name, spec.description);
    }

    let tachy_dir = env::current_dir().unwrap_or_default().join(".tachy");
    let custom = tools::CustomToolRegistry::load(&tachy_dir);
    if custom.tools().is_empty() {
        println!("\nNo custom tools defined. Create .tachy/tools.yaml to add custom tools.");
    } else {
        println!("\nCustom tools (.tachy/tools.yaml):\n");
        for tool in custom.tools() {
            let type_str = match tool.r#type {
                tools::custom::ToolType::Shell => "shell",
                tools::custom::ToolType::Http => "http",
            };
            let approval = if tool.approval_required { " [approval required]" } else { "" };
            println!("  {:20} {} ({}){}", tool.name, tool.description, type_str, approval);
        }
    }
}

pub(crate) fn list_models() {
    let registry = BackendRegistry::with_defaults();
    if json_output_enabled() {
        let models: Vec<serde_json::Value> = registry.list_models().iter().map(|m| {
            serde_json::json!({
                "name": m.name,
                "backend": format!("{:?}", m.backend),
                "context_window": m.context_window,
                "supports_tool_use": m.supports_tool_use,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&models).unwrap_or_default());
        return;
    }
    println!("Available models:\n");
    for model in registry.list_models() {
        let tools = if model.supports_tool_use { "tools" } else { "no-tools" };
        println!(
            "  {:30} {:12} ctx={:>7}  {}",
            model.name,
            format!("{:?}", model.backend),
            model.context_window,
            tools,
        );
    }
}

pub(crate) fn list_models_local() {
    let models = backend::discover_local_models("http://localhost:11434");
    if models.is_empty() {
        println!("No models installed locally.");
        println!("  Run: tachy pull llama3.1:8b");
        return;
    }
    println!("Locally installed models ({}):\n", models.len());
    for model in &models {
        println!(
            "  {:30} {:>8}  {:6}  {}",
            model.name,
            model.size_human(),
            model.parameter_size,
            model.quantization,
        );
    }
}

pub(crate) fn list_agents() {
    let config = PlatformConfig::default();
    if json_output_enabled() {
        let agents: Vec<serde_json::Value> = config.agent_templates.iter().map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "model": t.model,
                "max_iterations": t.max_iterations,
                "requires_approval": t.requires_approval,
                "allowed_tools": t.allowed_tools,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&agents).unwrap_or_default());
        return;
    }
    println!("Built-in agent templates:\n");
    for template in &config.agent_templates {
        println!("  {}", template.name);
        println!("    {}", template.description);
        println!("    model: {}  max_iterations: {}  approval: {}",
            template.model, template.max_iterations, template.requires_approval);
        println!("    tools: {}", template.allowed_tools.join(", "));
        println!();
    }
}
