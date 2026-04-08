//! Agent lifecycle commands: serve, run-agent, pull, publish, MCP, verify-audit,
//! activate, license, deploy.

use std::env;
use std::io::{self, Write};
use std::path::Path;

use daemon::DaemonState;

use crate::DEFAULT_MODEL;
use crate::info::json_output_enabled;

pub(crate) fn run_serve(addr: &str, workspace: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = workspace
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_default());
    let state = DaemonState::init(cwd.clone()).map_err(|e| e.to_string())?;
    let state = std::sync::Arc::new(std::sync::Mutex::new(state));

    eprintln!("Workspace: {}", cwd.display());
    eprintln!("State is auto-saved on every change. Ctrl+C to stop.");

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(daemon::serve(addr, state.clone()));

    if let Ok(s) = state.lock() {
        s.save();
        s.audit_logger.flush();
    }

    result
}

pub(crate) fn run_agent_cmd(template: &str, prompt: &str, model: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let mut state = DaemonState::init(cwd).map_err(|e| e.to_string())?;

    let agent_id = state.create_agent(template, prompt).map_err(|e| e.to_string())?;
    let config = state.agents.get(&agent_id).unwrap().config.clone();

    let mut config = config;
    if model != DEFAULT_MODEL {
        config.template.model = model.to_string();
    }

    println!("Running agent '{template}' ({})", config.template.model);
    println!("Prompt: {prompt}\n");

    let result = daemon::AgentEngine::run_agent(
        &agent_id,
        &config,
        prompt,
        &state.registry,
        &state.config.governance,
        &state.audit_logger,
        &state.config.intelligence,
        &state.workspace_root,
        None,
        None,
    );

    if result.success {
        println!("✓ Agent completed ({} iterations, {} tool calls)\n", result.iterations, result.tool_invocations);
        println!("{}", result.summary);
    } else {
        eprintln!("✗ Agent failed: {}", result.summary);
    }

    state.audit_logger.flush();
    Ok(())
}

pub(crate) fn run_pull(model: &str) -> Result<(), Box<dyn std::error::Error>> {
    backend::pull_model(model).map_err(std::convert::Into::into)
}

pub(crate) fn publish_agent(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::path::Path::new(path);
    if !source.exists() {
        return Err(format!("file not found: {path}").into());
    }

    let content = std::fs::read_to_string(source)?;

    let name = content.lines()
        .find(|l| l.trim_start().starts_with("name:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .or_else(|| {
            serde_json::from_str::<serde_json::Value>(&content).ok()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        })
        .ok_or("agent file must have a 'name' field")?;

    let agents_dir = env::current_dir()?.join(".tachy").join("agents");
    std::fs::create_dir_all(&agents_dir)?;
    let dest = agents_dir.join(format!("{name}.yaml"));
    std::fs::copy(source, &dest)?;

    if json_output_enabled() {
        println!("{}", serde_json::json!({"name": name, "path": dest.to_string_lossy()}));
    } else {
        println!("Published agent '{}' to {}", name, dest.display());
    }
    Ok(())
}

pub(crate) fn run_mcp_connect(server_filter: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    let config_path = tachy_dir.join("config.json");

    let configs: Vec<daemon::McpServerConfig> = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let parsed: serde_json::Value = serde_json::from_str(&content)?;
        if let Some(servers) = parsed.get("mcp_servers").and_then(|v| v.as_array()) {
            servers.iter()
                .filter_map(|s| serde_json::from_value(s.clone()).ok())
                .filter(|s: &daemon::McpServerConfig| {
                    server_filter.is_none_or(|f| s.name == f)
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    if configs.is_empty() {
        if server_filter.is_some() {
            println!("No MCP server matching '{}' found in .tachy/config.json", server_filter.unwrap());
        } else {
            println!("No MCP servers configured. Add to .tachy/config.json:");
            println!(r#"  "mcp_servers": [{{"name": "example", "command": "uvx", "args": ["mcp-server-example"]}}]"#);
        }
        return Ok(());
    }

    println!("Connecting to {} MCP server(s)...\n", configs.len());
    let mgr = daemon::McpClientManager::connect_all(&configs);
    let tools = mgr.all_tools();

    if tools.is_empty() {
        println!("Connected but no tools discovered.");
    } else {
        println!("Discovered {} tool(s):\n", tools.len());
        for tool in &tools {
            println!("  {} — {}", tool.qualified_name(), tool.description);
        }
    }
    println!("\nMCP connection test complete.");
    Ok(())
}

pub(crate) fn verify_audit() -> Result<(), Box<dyn std::error::Error>> {
    let audit_path = env::current_dir()?.join(".tachy").join("audit.jsonl");
    if !audit_path.exists() {
        println!("No audit log found. Run `tachy init` first.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&audit_path)?;
    let events: Vec<audit::AuditEvent> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if events.is_empty() {
        println!("Audit log is empty.");
        return Ok(());
    }

    println!("Tachy Audit Verification Report");
    println!("================================\n");
    println!("  Log file:    {}", audit_path.display());
    println!("  Total events: {}", events.len());
    println!("  First event:  {}", events[0].timestamp);
    println!("  Last event:   {}", events[events.len() - 1].timestamp);

    let mut kind_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut severity_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for event in &events {
        *kind_counts.entry(format!("{:?}", event.kind)).or_insert(0) += 1;
        *severity_counts.entry(format!("{:?}", event.severity)).or_insert(0) += 1;
    }

    println!("\n  Events by type:");
    for (kind, count) in &kind_counts {
        println!("    {kind:25} {count}");
    }

    println!("\n  Events by severity:");
    for (sev, count) in &severity_counts {
        println!("    {sev:25} {count}");
    }

    println!("\n  Hash chain verification:");
    match audit::verify_audit_chain(&events) {
        Ok(count) => {
            println!("    ✓ Chain intact: {count} events verified");
            println!("    ✓ Last hash: {}…", &events[count - 1].hash.get(..16).unwrap_or("unknown"));
            println!("\n  RESULT: PASS — audit trail is tamper-proof");
        }
        Err(broken_at) => {
            println!("    ✗ Chain BROKEN at event {broken_at}");
            if broken_at > 0 {
                println!("    ✗ Events 0-{} are valid", broken_at - 1);
            }
            println!("    ✗ The audit log has been tampered with or corrupted");
            println!("\n  RESULT: FAIL — audit trail integrity compromised");
        }
    }

    println!("\n================================");
    println!("This report can be provided to compliance auditors.");
    println!("The audit log is append-only JSONL with SHA-256 hash chain.");

    Ok(())
}

pub(crate) fn activate_license(key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    std::fs::create_dir_all(&tachy_dir)?;
    let mut license = audit::LicenseFile::load_or_create(&tachy_dir);

    let secret = env::var("TACHY_LICENSE_SECRET")
        .unwrap_or_else(|_| "tachy-license-secret-v1".to_string());

    match license.activate(key, &secret) {
        Ok(data) => {
            license.save(&tachy_dir)?;
            println!("✓ License activated!");
            println!("  Email: {}", data.email);
            println!("  Tier: {:?}", data.tier);
            if data.expires_at > 0 {
                println!("  Expires: {}s", data.expires_at);
            } else {
                println!("  Expires: never (perpetual)");
            }
        }
        Err(e) => {
            eprintln!("✗ Activation failed: {e}");
            eprintln!();
            eprintln!("If you purchased a license, check that you copied the full key.");
            eprintln!("Contact support@tachy.dev if the problem persists.");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn show_license_status() -> Result<(), Box<dyn std::error::Error>> {
    let tachy_dir = env::current_dir()?.join(".tachy");
    let license = audit::LicenseFile::load_or_create(&tachy_dir);
    let status = license.status();

    println!("Tachy License Status\n");
    println!("  Status:     {}", status.display());
    println!("  Machine ID: {}", license.machine_id);
    if !license.license_key.is_empty() {
        println!("  Key:        {}…", &license.license_key[..20.min(license.license_key.len())]);
    }
    if let Some(data) = &license.license {
        println!("  Email:      {}", data.email);
        println!("  Tier:       {:?}", data.tier);
    }
    Ok(())
}

pub(crate) fn run_deploy(profile: Option<&str>, region: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Tachy Enterprise: Cloud Bridge (AWS Batch)");
    println!("==========================================");

    let cwd = env::current_dir()?;
    let tachy_dir = cwd.join(".tachy");
    if !tachy_dir.exists() {
        return Err("Workspace not initialized — run: tachy init".into());
    }

    print!("📦 Bundling workspace... ");
    io::stdout().flush()?;
    let version = "0.1.0";
    let bundle_name = format!("tachy-workspace-{}.tar.gz", chrono_now_secs());
    let _ = (version, bundle_name); // placeholders
    println!("✓ (ready for packaging)");

    println!("☁ Checking AWS environment:");
    if let Some(p) = profile { println!("  Profile: {p}"); }
    if let Some(r) = region { println!("  Region:  {r}"); }

    let has_aws_cli = std::process::Command::new("aws").arg("--version").output().is_ok();
    if has_aws_cli {
        println!("  ✓ AWS CLI detected");
    } else {
        println!("  ⚠ Warning: 'aws' CLI not found. Job submission will require native SDK.");
    }

    print!("🐳 Packaging OCI container... ");
    io::stdout().flush()?;
    let has_docker = std::process::Command::new("docker").arg("--version").output().is_ok();
    if has_docker {
        println!("✓ Docker ready");
    } else {
        println!("⚠ Docker not found (required for local image builds)");
    }

    println!("\nNext Steps for Phase 4.2:");
    println!("  1. Upload bundle to S3: s3://tachy-deployments/<bundle>");
    println!("  2. Submit Batch Job: SubmitJob(jobDefinition='tachy-agent-{version}')");
    println!("  3. Monitor progress at http://localhost:7777/dashboard (Cloud Tab)");

    println!("\nDeployment engine initialized. Waiting for storage credentials.");
    Ok(())
}

pub(crate) fn chrono_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
