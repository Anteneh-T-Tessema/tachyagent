//! `tachy pipeline` — YAML-defined multi-step agent pipelines with DAG scheduling.

use crate::DEFAULT_MODEL;
use crate::agent::run_agent_cmd;

/// A single step in a pipeline.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct PipelineStep {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

/// A pipeline definition loaded from YAML.
#[derive(Debug, serde::Deserialize)]
struct PipelineDefinition {
    name: String,
    #[serde(default)]
    description: String,
    steps: Vec<PipelineStep>,
}

pub(crate) fn run_pipeline(subcommand: &str, path: &str, dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    if subcommand == "init" {
        let target = if path == "pipeline.yaml" { "tachy-pipeline.yaml" } else { path };
        if std::path::Path::new(target).exists() {
            return Err(format!("{target} already exists — remove it first").into());
        }
        let template = r#"name: my-pipeline
description: "A multi-step agent pipeline"

steps:
  - name: review
    template: code-reviewer
    prompt: "Review the code in the current directory for quality and correctness."

  - name: security
    template: security-scanner
    prompt: "Scan the codebase for security vulnerabilities."
    depends_on: [review]

  - name: docs
    template: doc-generator
    prompt: "Generate or update documentation for all public APIs."
    depends_on: [review]
"#;
        std::fs::write(target, template)?;
        println!("Created {target}");
        println!("Edit the steps then run: tachy pipeline run {target}");
        return Ok(());
    }

    if subcommand != "run" && subcommand != "validate" {
        return Err(format!("unknown pipeline subcommand: {subcommand}\n  usage: tachy pipeline run|validate|init <pipeline.yaml>").into());
    }

    let yaml_str = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read pipeline file '{path}': {e}"))?;

    let pipeline: PipelineDefinition = serde_yaml::from_str(&yaml_str)
        .map_err(|e| format!("invalid pipeline YAML: {e}"))?;

    let step_names: std::collections::HashSet<&str> = pipeline.steps.iter().map(|s| s.name.as_str()).collect();
    for step in &pipeline.steps {
        for dep in &step.depends_on {
            if !step_names.contains(dep.as_str()) {
                return Err(format!("step '{}' depends_on '{}' which does not exist", step.name, dep).into());
            }
        }
    }

    topological_sort(&pipeline.steps)
        .map_err(|cycle| format!("pipeline has a dependency cycle: {cycle}"))?;

    print_pipeline_dag(&pipeline.steps);

    if subcommand == "validate" || dry_run {
        println!("Pipeline '{}' is valid ({} steps).", pipeline.name, pipeline.steps.len());
        return Ok(());
    }

    println!("Running pipeline: {}", pipeline.name);
    if !pipeline.description.is_empty() {
        println!("  {}", pipeline.description);
    }
    println!();

    let order = topological_sort(&pipeline.steps).unwrap();
    let default_model = DEFAULT_MODEL.to_string();

    for step_name in &order {
        let step = pipeline.steps.iter().find(|s| &s.name == step_name).unwrap();
        let model = step.model.as_deref().unwrap_or(&default_model);
        println!("  ► Step '{}' — template: {}, model: {}", step.name, step.template, model);
        if dry_run {
            continue;
        }
        match run_agent_cmd(&step.template, &step.prompt, model) {
            Ok(()) => println!("    ✓ Step '{}' completed\n", step.name),
            Err(e) => {
                eprintln!("    ✗ Step '{}' failed: {e}", step.name);
                return Err(format!("pipeline aborted at step '{}'", step.name).into());
            }
        }
    }

    println!("Pipeline '{}' completed all {} steps.", pipeline.name, pipeline.steps.len());
    Ok(())
}

/// Render an ASCII DAG of pipeline steps showing dependency arrows.
fn print_pipeline_dag(steps: &[PipelineStep]) {
    println!("Pipeline DAG:");
    let mut children: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    let mut has_parent: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for step in steps {
        for dep in &step.depends_on {
            children.entry(dep.as_str()).or_default().push(step.name.as_str());
            has_parent.insert(step.name.as_str());
        }
    }

    let roots: Vec<&str> = steps.iter()
        .filter(|s| !has_parent.contains(s.name.as_str()))
        .map(|s| s.name.as_str())
        .collect();

    fn render(name: &str, indent: usize, children: &std::collections::BTreeMap<&str, Vec<&str>>) {
        let pad = "  ".repeat(indent);
        println!("{pad}[{name}]");
        if let Some(kids) = children.get(name) {
            let n = kids.len();
            for (i, kid) in kids.iter().enumerate() {
                let connector = if i + 1 == n { "└─▶" } else { "├─▶" };
                let kid_pad = "  ".repeat(indent + 1);
                println!("{kid_pad}{connector} [{kid}]");
                render(kid, indent + 2, children);
            }
        }
    }

    for root in &roots {
        render(root, 1, &children);
    }
    if roots.is_empty() {
        for step in steps {
            println!("  [{}]", step.name);
        }
    }
    println!();
}

/// Topological sort of pipeline steps. Returns ordered step names or an error
/// describing the cycle.
pub(crate) fn topological_sort(steps: &[PipelineStep]) -> Result<Vec<String>, String> {
    let mut count: std::collections::HashMap<&str, usize> = steps.iter()
        .map(|s| (s.name.as_str(), s.depends_on.len()))
        .collect();

    let mut queue: std::collections::VecDeque<&str> = count.iter()
        .filter(|(_, &c)| c == 0)
        .map(|(&n, _)| n)
        .collect();

    let mut result = Vec::new();
    while let Some(name) = queue.pop_front() {
        result.push(name.to_string());
        for step in steps {
            if step.depends_on.iter().any(|d| d == name) {
                let c = count.entry(step.name.as_str()).or_default();
                *c = c.saturating_sub(1);
                if *c == 0 {
                    queue.push_back(step.name.as_str());
                }
            }
        }
    }

    if result.len() == steps.len() {
        Ok(result)
    } else {
        Err("cycle detected".to_string())
    }
}
