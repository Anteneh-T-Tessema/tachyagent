mod input;
mod render;

mod agent;
mod analysis;
mod args;
mod doctor;
mod info;
mod pipeline;
mod repl;
mod setup;

use std::env;

use args::{CliAction, parse_args};

const DEFAULT_MODEL: &str = "gemma4:26b";
const DEFAULT_DATE: &str = "2026-04-03";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let action = parse_args(&args)?;

    // License check — skip for non-agent commands
    let needs_license = matches!(
        action,
        CliAction::Prompt { .. } | CliAction::Repl { .. } | CliAction::RunAgent { .. } | CliAction::Serve { .. }
    );
    if needs_license {
        let tachy_dir = env::current_dir()
            .unwrap_or_default()
            .join(".tachy");
        let license = audit::LicenseFile::load_or_create(&tachy_dir);
        let status = license.status();
        if !status.is_active() {
            eprintln!("⚠ {}", status.display());
            eprintln!();
            eprintln!("Purchase a license at https://tachy.dev/pricing");
            eprintln!("Then run: tachy activate <LICENSE_KEY>");
            std::process::exit(1);
        }
        if let audit::LicenseStatus::TrialActive { remaining_secs } = &status {
            let days = remaining_secs / 86400;
            if days <= 2 {
                eprintln!("⚠ Trial expires in {} — https://tachy.dev/pricing", status.display());
            }
        }
    }

    match action {
        CliAction::BootstrapPlan => setup::print_bootstrap_plan(),
        CliAction::PrintSystemPrompt { cwd, date } => setup::print_system_prompt(cwd, date),
        CliAction::ResumeSession { session_path, command } => setup::resume_session(&session_path, command),
        CliAction::Prompt { prompt, model } => {
            let mut cli = repl::LiveCli::new(model, true)?;
            cli.run_turn(&prompt)?;
        }
        CliAction::Repl { model } => repl::run_repl(model)?,
        CliAction::Init => setup::init_workspace()?,
        CliAction::Setup => setup::run_setup_wizard()?,
        CliAction::ListModels => info::list_models(),
        CliAction::ListModelsLocal => info::list_models_local(),
        CliAction::ListAgents => info::list_agents(),
        CliAction::YayaPreferences { workspace, subject, set_sources, set_terms, clear } => {
            info::handle_yaya_preferences(&workspace, &subject, set_sources.as_deref(), set_terms.as_deref(), clear)?
        }
        CliAction::Serve { addr, workspace } => agent::run_serve(&addr, workspace.as_deref()).await?,
        CliAction::RunAgent { template, prompt, model } => agent::run_agent_cmd(&template, &prompt, &model)?,
        CliAction::Doctor { json } => doctor::run_doctor(json),
        CliAction::Pull { model } => agent::run_pull(&model)?,
        CliAction::VerifyAudit => agent::verify_audit()?,
        CliAction::Warmup { model } => setup::warmup_model(&model)?,
        CliAction::InstallOllama => {
            match setup::install_ollama() {
                Ok(()) => {
                    println!("✓ Ollama installed");
                    println!("Starting server...");
                    let _ = setup::start_ollama();
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    if backend::check_ollama("http://localhost:11434").0 {
                        println!("✓ Ollama server running");
                    } else {
                        println!("Server not responding yet — try: ollama serve");
                    }
                }
                Err(e) => eprintln!("✗ Install failed: {e}\n  Install manually: https://ollama.com/download"),
            }
        }
        CliAction::ListTools => info::list_tools(),
        CliAction::ListChannels => info::list_channels(),
        CliAction::McpServer => daemon::run_mcp_server(),
        CliAction::McpConnect { server } => agent::run_mcp_connect(server.as_deref())?,
        CliAction::PublishAgent { path } => agent::publish_agent(&path)?,
        CliAction::Deploy { profile, region } => agent::run_deploy(profile.as_deref(), region.as_deref())?,
        CliAction::Activate { key } => agent::activate_license(&key)?,
        CliAction::LicenseStatus => agent::show_license_status()?,
        CliAction::Help => analysis::print_help(),
        CliAction::Search { query, limit } => analysis::run_search(&query, limit)?,
        CliAction::Pipeline { subcommand, path, dry_run } => pipeline::run_pipeline(&subcommand, &path, dry_run)?,
        CliAction::Graph { format, file } => analysis::run_graph(&format, file.as_deref())?,
        CliAction::Monorepo => analysis::run_monorepo()?,
        CliAction::Dashboard => analysis::run_dashboard()?,
        CliAction::ExportAudit { format, output } => analysis::run_export_audit(&format, output.as_deref())?,
        CliAction::Finetune { output, base_model } => analysis::run_finetune(output.as_deref(), base_model.as_deref())?,
        CliAction::Policy { subcommand, file } => analysis::run_policy(&subcommand, file.as_deref())?,
        CliAction::Swarm { subcommand, goal, url, files, model } => {
            analysis::run_swarm(&subcommand, goal.as_deref(), url.as_deref(), &files, model.as_deref())?
        }
        CliAction::OptimizeBrain => analysis::run_optimize_brain()?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{parse_args, CliAction};
    use crate::pipeline::{PipelineStep, topological_sort};
    use crate::analysis::urlencoding_simple;
    use std::path::PathBuf;

    #[test]
    fn defaults_to_repl_when_no_args() {
        assert_eq!(
            parse_args(&[]).expect("args should parse"),
            CliAction::Repl { model: None },
        );
    }

    #[test]
    fn parses_prompt_subcommand() {
        let args = vec!["prompt".to_string(), "hello".to_string(), "world".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Prompt { prompt: "hello world".to_string(), model: DEFAULT_MODEL.to_string() },
        );
    }

    #[test]
    fn parses_init_command() {
        let args = vec!["init".to_string()];
        assert_eq!(parse_args(&args).expect("args should parse"), CliAction::Init);
    }

    #[test]
    fn parses_models_command() {
        let args = vec!["models".to_string()];
        assert_eq!(parse_args(&args).expect("args should parse"), CliAction::ListModels);
    }

    #[test]
    fn parses_model_override() {
        let args = vec!["--model".to_string(), "qwen2.5-coder:7b".to_string(), "prompt".to_string(), "hello".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Prompt { prompt: "hello".to_string(), model: "qwen2.5-coder:7b".to_string() },
        );
    }

    #[test]
    fn explicit_model_repl_preserves_model() {
        let args = vec!["--model".to_string(), "qwen2.5-coder:7b".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Repl { model: Some("qwen2.5-coder:7b".to_string()) },
        );
    }

    #[test]
    fn parses_resume_flag_with_slash_command() {
        let args = vec!["--resume".to_string(), "session.json".to_string(), "/compact".to_string()];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::ResumeSession {
                session_path: PathBuf::from("session.json"),
                command: Some("/compact".to_string()),
            },
        );
    }

    #[test]
    fn parses_search_single_word() {
        let args = vec!["search".to_string(), "main".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Search { query, limit } => {
                assert_eq!(query, "main");
                assert_eq!(limit, 10);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_search_multi_word_query() {
        let args = vec!["search".to_string(), "tool".to_string(), "calling".to_string(), "rust".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Search { query, .. } => assert_eq!(query, "tool calling rust"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn search_empty_query_returns_error() {
        let args = vec!["search".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn parses_pipeline_run() {
        let args = vec!["pipeline".to_string(), "run".to_string(), "my-pipeline.yaml".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, path, dry_run } => {
                assert_eq!(subcommand, "run");
                assert_eq!(path, "my-pipeline.yaml");
                assert!(!dry_run);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_run_dry_run() {
        let args = vec!["pipeline".to_string(), "run".to_string(), "pipe.yaml".to_string(), "--dry-run".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { dry_run, .. } => assert!(dry_run),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_init() {
        let args = vec!["pipeline".to_string(), "init".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, .. } => assert_eq!(subcommand, "init"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_pipeline_validate() {
        let args = vec!["pipeline".to_string(), "validate".to_string(), "ci.yaml".to_string()];
        match parse_args(&args).expect("should parse") {
            CliAction::Pipeline { subcommand, path, .. } => {
                assert_eq!(subcommand, "validate");
                assert_eq!(path, "ci.yaml");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn topo_sort_no_deps() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(), depends_on: vec![], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(), depends_on: vec![], model: None },
        ];
        let order = topological_sort(&steps).expect("valid");
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn topo_sort_linear_chain() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(), depends_on: vec![], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(), depends_on: vec!["a".into()], model: None },
            PipelineStep { name: "c".into(), template: "t".into(), prompt: "p".into(), depends_on: vec!["b".into()], model: None },
        ];
        let order = topological_sort(&steps).expect("valid");
        assert_eq!(order.len(), 3);
        let ia = order.iter().position(|x| x == "a").unwrap();
        let ib = order.iter().position(|x| x == "b").unwrap();
        let ic = order.iter().position(|x| x == "c").unwrap();
        assert!(ia < ib && ib < ic);
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let steps = vec![
            PipelineStep { name: "a".into(), template: "t".into(), prompt: "p".into(), depends_on: vec!["b".into()], model: None },
            PipelineStep { name: "b".into(), template: "t".into(), prompt: "p".into(), depends_on: vec!["a".into()], model: None },
        ];
        assert!(topological_sort(&steps).is_err());
    }

    #[test]
    fn urlencoding_leaves_alphanumeric() {
        assert_eq!(urlencoding_simple("hello"), "hello");
        assert_eq!(urlencoding_simple("Rust2024"), "Rust2024");
    }

    #[test]
    fn urlencoding_encodes_space_as_plus() {
        assert_eq!(urlencoding_simple("hello world"), "hello+world");
    }

    #[test]
    fn urlencoding_encodes_special_chars() {
        let encoded = urlencoding_simple("a=b&c");
        assert!(encoded.contains("%3D") || encoded.contains("%3d"));
        assert!(encoded.contains("%26"));
    }
}
