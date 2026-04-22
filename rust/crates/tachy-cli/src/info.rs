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

fn daemon_url() -> String {
    env::var("TACHY_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:7777".to_string())
}

fn daemon_api_key() -> Option<String> {
    env::var("TACHY_API_KEY").ok().filter(|value| !value.trim().is_empty())
}

fn load_yaya_preferences_json(workspace: &str, subject: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut command = std::process::Command::new("curl");
    command.arg("-sf");
    if let Some(api_key) = daemon_api_key() {
        command.args(["-H", &format!("Authorization: Bearer {api_key}")]);
    }
    command.arg(format!(
        "{}/api/yaya/retrieval-preferences?workspace={}&subject={}",
        daemon_url(),
        crate::analysis::urlencoding_simple(workspace),
        crate::analysis::urlencoding_simple(subject),
    ));

    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("failed to load yaya retrieval preferences: {}{}", stdout, stderr).into());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(serde_json::from_str(&text)?)
}

fn save_yaya_preferences_json(
    workspace: &str,
    subject: &str,
    preferred_sources: &[String],
    preferred_terms: &[String],
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let payload = serde_json::json!({
        "workspace": workspace,
        "subject": subject,
        "preferred_sources": preferred_sources,
        "preferred_source_terms": preferred_terms,
    });

    let mut command = std::process::Command::new("curl");
    command.args(["-sf", "-X", "POST", "-H", "Content-Type: application/json"]);
    if let Some(api_key) = daemon_api_key() {
        command.args(["-H", &format!("Authorization: Bearer {api_key}")]);
    }
    command.args([
        "-d",
        &payload.to_string(),
        &format!("{}/api/yaya/retrieval-preferences", daemon_url()),
    ]);

    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("failed to save yaya retrieval preferences: {}{}", stdout, stderr).into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(serde_json::from_str(&text)?)
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn handle_yaya_preferences(
    workspace: &str,
    subject: &str,
    set_sources: Option<&str>,
    set_terms: Option<&str>,
    clear: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if clear || set_sources.is_some() || set_terms.is_some() {
        let current = load_yaya_preferences_json(workspace, subject)?;
        let mut sources = current
            .get("explicit_preferred_sources")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(|v| v.as_str()).map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut terms = current
            .get("explicit_preferred_source_terms")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(|v| v.as_str()).map(ToString::to_string).collect::<Vec<_>>())
            .unwrap_or_default();

        if clear {
            sources.clear();
            terms.clear();
        }
        if let Some(value) = set_sources {
            sources = parse_csv_list(value);
        }
        if let Some(value) = set_terms {
            terms = parse_csv_list(value);
        }
        let saved = save_yaya_preferences_json(workspace, subject, &sources, &terms)?;
        if json_output_enabled() {
            println!("{}", serde_json::to_string_pretty(&saved)?);
        } else {
            println!("Updated Yaya retrieval preferences for {workspace}/{subject}.\n");
        }
    }
    show_yaya_preferences(workspace, subject)
}

pub(crate) fn show_yaya_preferences(workspace: &str, subject: &str) -> Result<(), Box<dyn std::error::Error>> {
    let json = load_yaya_preferences_json(workspace, subject)?;
    if json_output_enabled() {
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    println!("Yaya retrieval preferences\n");
    println!("  Workspace: {}", json.get("workspace").and_then(|v| v.as_str()).unwrap_or(workspace));
    println!("  Subject:   {}", json.get("subject").and_then(|v| v.as_str()).unwrap_or(subject));
    println!("  Strategy:  {}", json.get("strategy").and_then(|v| v.as_str()).unwrap_or("workspace_wide"));
    println!(
        "  Updated:   {}",
        json.get("updated_at").and_then(|v| v.as_str()).unwrap_or("inferred only")
    );
    println!(
        "  Approved examples: {}",
        json.get("approved_example_count").and_then(|v| v.as_u64()).unwrap_or(0)
    );

    let print_list = |label: &str, key: &str| {
        println!("\n  {label}:");
        if let Some(items) = json.get(key).and_then(|v| v.as_array()) {
            if items.is_empty() {
                println!("    - none");
            } else {
                for item in items.iter().filter_map(|v| v.as_str()) {
                    println!("    - {item}");
                }
            }
        } else {
            println!("    - none");
        }
    };

    print_list("Effective sources", "preferred_sources");
    print_list("Effective terms", "preferred_source_terms");
    print_list("Explicit sources", "explicit_preferred_sources");
    print_list("Explicit terms", "explicit_preferred_source_terms");
    print_list("Inferred sources", "inferred_preferred_sources");
    print_list("Inferred terms", "inferred_preferred_source_terms");

    Ok(())
}
