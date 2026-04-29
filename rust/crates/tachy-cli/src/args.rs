//! CLI argument parsing: `CliAction` enum + `parse_args` + sub-parsers.

use std::path::PathBuf;

use crate::DEFAULT_MODEL;
use crate::repl::last_session_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
    BootstrapPlan,
    PrintSystemPrompt {
        cwd: PathBuf,
        date: String,
    },
    ResumeSession {
        session_path: PathBuf,
        command: Option<String>,
    },
    Prompt {
        prompt: String,
        model: String,
    },
    Repl {
        model: Option<String>,
    },
    Init,
    Setup,
    ListModels,
    ListModelsLocal,
    ListAgents,
    YayaPreferences {
        workspace: String,
        subject: String,
        set_sources: Option<String>,
        set_terms: Option<String>,
        clear: bool,
    },
    Serve { addr: String, workspace: Option<PathBuf> },
    RunAgent { template: String, prompt: String, model: String },
    Doctor { json: bool },
    Pull { model: String },
    VerifyAudit,
    Warmup { model: String },
    InstallOllama,
    ListTools,
    ListChannels,
    McpServer,
    McpConnect { server: Option<String> },
    PublishAgent { path: String },
    Activate { key: String },
    Deploy { profile: Option<String>, region: Option<String> },
    LicenseStatus,
    Help,
    Search { query: String, limit: usize },
    Pipeline { subcommand: String, path: String, dry_run: bool },
    /// C1: dependency/call graph
    Graph { format: String, file: Option<String> },
    /// C3: monorepo workspace detection
    Monorepo,
    /// E2: dashboard metrics
    Dashboard,
    /// F1: export audit log in various compliance formats
    ExportAudit { format: String, output: Option<String> },
    /// F3: fine-tuning dataset extraction + `LoRA` script generation
    Finetune { output: Option<String>, base_model: Option<String> },
    /// F2: policy-as-code — view/set/validate tachy-policy.yaml
    Policy { subcommand: String, file: Option<String> },
    /// Phase 23: swarm — manage distributed nodes and run multi-agent tasks
    Swarm { 
        subcommand: String, 
        goal: Option<String>, 
        url: Option<String>, 
        files: Vec<String>, 
        model: Option<String> 
    },
    /// Phase 21: optimize — trigger autonomous fine-tuning bridge
    OptimizeBrain,
}

pub(crate) fn parse_args(args: &[String]) -> Result<CliAction, String> {
    let mut model = DEFAULT_MODEL.to_string();
    let mut model_explicit = false;
    let mut rest = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--model" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --model".to_string())?;
                model = value.clone();
                model_explicit = true;
                index += 2;
            }
            flag if flag.starts_with("--model=") => {
                model = flag[8..].to_string();
                model_explicit = true;
                index += 1;
            }
            "--json" => {
                std::env::set_var("TACHY_OUTPUT_JSON", "1");
                index += 1;
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }

    if rest.is_empty() {
        return Ok(CliAction::Repl { model: if model_explicit { Some(model) } else { None } });
    }
    if matches!(rest.first().map(String::as_str), Some("--help" | "-h")) {
        return Ok(CliAction::Help);
    }
    if rest.first().map(String::as_str) == Some("--resume") {
        return parse_resume_args(&rest[1..]);
    }
    if rest.first().map(String::as_str) == Some("--continue") {
        if let Some(path) = last_session_path() {
            return Ok(CliAction::ResumeSession {
                session_path: path,
                command: None,
            });
        }
        return Err("no previous session found in .tachy/sessions/".to_string());
    }

    match rest[0].as_str() {
        "bootstrap-plan" => Ok(CliAction::BootstrapPlan),
        "system-prompt" => parse_system_prompt_args(&rest[1..]),
        "init" => Ok(CliAction::Init),
        "setup" => Ok(CliAction::Setup),
        "models" => {
            if rest.get(1).map(String::as_str) == Some("--local") {
                Ok(CliAction::ListModelsLocal)
            } else {
                Ok(CliAction::ListModels)
            }
        }
        "agents" => Ok(CliAction::ListAgents),
        "yaya-preferences" => {
            let mut workspace = "default".to_string();
            let mut subject = None;
            let mut set_sources = None;
            let mut set_terms = None;
            let mut clear = false;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--workspace" | "-w" => {
                        workspace = rest.get(i + 1).cloned().unwrap_or_else(|| "default".to_string());
                        i += 2;
                    }
                    "--subject" | "-s" => {
                        subject = rest.get(i + 1).cloned();
                        i += 2;
                    }
                    "--set-sources" => {
                        set_sources = rest.get(i + 1).cloned();
                        i += 2;
                    }
                    "--set-terms" => {
                        set_terms = rest.get(i + 1).cloned();
                        i += 2;
                    }
                    "--clear" => {
                        clear = true;
                        i += 1;
                    }
                    other if !other.starts_with("--") && subject.is_none() => {
                        subject = Some(other.to_string());
                        i += 1;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let subject = subject.ok_or("usage: yaya-preferences --subject <subject> [--workspace <workspace>]")?;
            Ok(CliAction::YayaPreferences { workspace, subject, set_sources, set_terms, clear })
        }
        "doctor" => {
            let json = rest.iter().any(|a| a == "--json");
            Ok(CliAction::Doctor { json })
        }
        "verify-audit" => Ok(CliAction::VerifyAudit),
        "warmup" => {
            let warmup_model = rest.get(1).cloned().unwrap_or_else(|| model.clone());
            Ok(CliAction::Warmup { model: warmup_model })
        }
        "install-ollama" => Ok(CliAction::InstallOllama),
        "tools" => Ok(CliAction::ListTools),
        "channels" => Ok(CliAction::ListChannels),
        "mcp-server" => Ok(CliAction::McpServer),
        "mcp-connect" => {
            let server = rest.get(1).map(std::string::ToString::to_string);
            Ok(CliAction::McpConnect { server })
        }
        "publish" => {
            let path = rest.get(1).ok_or("usage: publish <agent.yaml>")?;
            Ok(CliAction::PublishAgent { path: path.to_string() })
        }
        "activate" => {
            let key = rest.get(1).ok_or("usage: activate <LICENSE_KEY>")?;
            Ok(CliAction::Activate { key: key.clone() })
        }
        "license" => Ok(CliAction::LicenseStatus),
        "pull" => {
            let model_name = rest.get(1).ok_or("usage: pull <model>")?;
            Ok(CliAction::Pull { model: model_name.clone() })
        }
        "deploy" => {
            let mut profile = None;
            let mut region = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--profile" | "-p" => { profile = rest.get(i+1).cloned(); i += 2; }
                    "--region" | "-r" => { region = rest.get(i+1).cloned(); i += 2; }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::Deploy { profile, region })
        }
        "serve" => {
            let addr = rest.get(1).cloned().unwrap_or_else(|| "127.0.0.1:7777".to_string());
            let workspace = rest.iter().enumerate()
                .find(|(_, a)| *a == "--workspace" || *a == "-w")
                .and_then(|(i, _)| rest.get(i + 1))
                .map(PathBuf::from);
            Ok(CliAction::Serve { addr, workspace })
        }
        "run-agent" => {
            let template = rest.get(1).ok_or("usage: run-agent <template> <prompt...>")?.clone();
            let prompt = if rest.len() > 2 { rest[2..].join(" ") } else { String::new() };
            Ok(CliAction::RunAgent { template, prompt, model })
        }
        "prompt" => {
            if rest.len() < 2 {
                return Err("usage: prompt <text...>".to_string());
            }
            Ok(CliAction::Prompt { prompt: rest[1..].join(" "), model })
        }
        "search" => {
            if rest.len() < 2 {
                return Err("usage: search <query> [--limit N]".to_string());
            }
            let mut limit = 10usize;
            let mut query_parts = Vec::new();
            let mut i = 1;
            while i < rest.len() {
                if rest[i] == "--limit" || rest[i] == "-n" {
                    limit = rest.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(10);
                    i += 2;
                } else {
                    query_parts.push(rest[i].as_str());
                    i += 1;
                }
            }
            if query_parts.is_empty() {
                return Err("usage: search <query> [--limit N]".to_string());
            }
            Ok(CliAction::Search { query: query_parts.join(" "), limit })
        }
        "pipeline" => {
            let subcommand = rest.get(1).cloned().unwrap_or_else(|| "run".to_string());
            let path = rest.get(2).cloned().unwrap_or_else(|| "pipeline.yaml".to_string());
            let dry_run = rest.iter().any(|a| a == "--dry-run");
            Ok(CliAction::Pipeline { subcommand, path, dry_run })
        }
        "graph" => {
            let mut format = "summary".to_string();
            let mut file = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--format" => { format = rest.get(i+1).cloned().unwrap_or_else(|| "summary".to_string()); i += 2; }
                    "--file" | "-f" => { file = rest.get(i+1).cloned(); i += 2; }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::Graph { format, file })
        }
        "monorepo" => Ok(CliAction::Monorepo),
        "dashboard" => Ok(CliAction::Dashboard),
        "export-audit" => {
            let mut format = "json".to_string();
            let mut output = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--format" => { format = rest.get(i+1).cloned().unwrap_or_else(|| "json".to_string()); i += 2; }
                    "--output" | "-o" => { output = rest.get(i+1).cloned(); i += 2; }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::ExportAudit { format, output })
        }
        "finetune" => {
            let mut output = None;
            let mut base_model = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--output" | "-o" => { output = rest.get(i+1).cloned(); i += 2; }
                    "--base-model" => { base_model = rest.get(i+1).cloned(); i += 2; }
                    _ => { i += 1; }
                }
            }
            Ok(CliAction::Finetune { output, base_model })
        }
        "policy" => {
            let subcommand = rest.get(1).cloned().unwrap_or_else(|| "show".to_string());
            let file = rest.iter().enumerate()
                .find(|(_, a)| *a == "--file" || *a == "-f")
                .and_then(|(i, _)| rest.get(i + 1))
                .cloned();
            Ok(CliAction::Policy { subcommand, file })
        }
        "swarm" => {
            let subcommand = rest.get(1).cloned().unwrap_or_else(|| "status".to_string());
            let mut goal = None;
            let mut url = None;
            let mut files = Vec::new();
            let mut swarm_model = None;
            
            if subcommand == "register" {
                url = rest.get(2).cloned();
            } else if subcommand == "run" {
                let mut i = 2;
                while i < rest.len() {
                    match rest[i].as_str() {
                        "--goal" | "-g" => { goal = rest.get(i+1).cloned(); i += 2; }
                        "--model" => { swarm_model = rest.get(i+1).cloned(); i += 2; }
                        other if other.starts_with("--") => { i += 1; }
                        other => { files.push(other.to_string()); i += 1; }
                    }
                }
                if goal.is_none() && !files.is_empty() {
                    goal = Some(files.remove(0));
                }
            } else {
                // status, list, etc.
            }
            
            Ok(CliAction::Swarm { subcommand, goal, url, files, model: swarm_model })
        }
        "optimize" => Ok(CliAction::OptimizeBrain),
        other => Err(format!("unknown command: {other}\nRun `tachy --help` for usage.")),
    }
}

fn parse_system_prompt_args(args: &[String]) -> Result<CliAction, String> {
    let cwd = args
        .iter()
        .enumerate()
        .find(|(_, a)| *a == "--cwd")
        .and_then(|(i, _)| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or("could not determine working directory")?;

    let date = args
        .iter()
        .enumerate()
        .find(|(_, a)| *a == "--date")
        .and_then(|(i, _)| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| crate::DEFAULT_DATE.to_string());

    Ok(CliAction::PrintSystemPrompt { cwd, date })
}

fn parse_resume_args(args: &[String]) -> Result<CliAction, String> {
    let session_path = args
        .first()
        .map(PathBuf::from)
        .ok_or("usage: --resume <session.json> [/command]")?;

    let command = args.get(1).cloned();

    Ok(CliAction::ResumeSession {
        session_path,
        command,
    })
}
