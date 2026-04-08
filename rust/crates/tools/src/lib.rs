use runtime::{
    edit_file, execute_bash, glob_search, grep_search, list_directory, read_file, write_file,
    BashCommandInput, DiffPreview, GrepSearchInput,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub mod custom;
pub mod web;
pub use custom::{CustomTool, CustomToolRegistry, CustomToolsFile};
pub use web::{web_search, web_fetch, WebSearchInput, WebSearchOutput, WebFetchInput, WebFetchOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifestEntry {
    pub name: String,
    pub source: ToolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSource {
    Base,
    Conditional,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRegistry {
    entries: Vec<ToolManifestEntry>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new(entries: Vec<ToolManifestEntry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[ToolManifestEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[must_use]
pub fn mvp_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bash",
            description: "Execute a shell command in the current workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "integer", "minimum": 1 },
                    "description": { "type": "string" },
                    "run_in_background": { "type": "boolean" },
                    "dangerouslyDisableSandbox": { "type": "boolean" }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "read_file",
            description: "Read a text file from the workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "write_file",
            description: "Write a text file in the workspace.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "edit_file",
            description: "Replace text in a workspace file.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "glob_search",
            description: "Find files by glob pattern.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "grep_search",
            description: "Search file contents with a regex pattern.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob": { "type": "string" },
                    "output_mode": { "type": "string" },
                    "-B": { "type": "integer", "minimum": 0 },
                    "-A": { "type": "integer", "minimum": 0 },
                    "-C": { "type": "integer", "minimum": 0 },
                    "context": { "type": "integer", "minimum": 0 },
                    "-n": { "type": "boolean" },
                    "-i": { "type": "boolean" },
                    "type": { "type": "string" },
                    "head_limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "multiline": { "type": "boolean" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_directory",
            description: "List files and directories at a given path. Returns names, types, and sizes. Skips node_modules, .git, target, and other noise directories.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to list (default: current directory)" },
                    "depth": { "type": "integer", "minimum": 1, "maximum": 5, "description": "How deep to recurse (default: 1)" }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "remember",
            description: "Store a persistent memory that survives across sessions. Use this to remember user preferences, project context, important decisions, or patterns you've learned.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "What to remember" },
                    "category": { "type": "string", "enum": ["preference", "project", "decision", "pattern", "note"], "description": "Category of memory" }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "call_agent",
            description: "Call another Tachy agent and get its result. Use this to orchestrate multi-agent workflows — e.g., call the code-reviewer, then the test-runner, then deploy.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "template": { "type": "string", "description": "Agent template name (e.g., 'code-reviewer', 'test-runner', 'security-scanner')" },
                    "prompt": { "type": "string", "description": "What to ask the agent to do" }
                },
                "required": ["template", "prompt"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "web_search",
            description: "Search the web and return results with titles, URLs, and snippets. Use this to find documentation, look up error messages, or research solutions.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 10, "description": "Max results to return (default: 5)" }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "web_fetch",
            description: "Fetch a URL and extract its text content. Use this to read documentation pages, API references, or any web page.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch (must start with http:// or https://)" },
                    "max_length": { "type": "integer", "minimum": 100, "maximum": 50000, "description": "Max content length in characters (default: 8000)" }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
        // E1: Richer diagnostics — LSP-powered error/warning extraction
        ToolSpec {
            name: "get_diagnostics",
            description: concat!(
                "Get language-server diagnostics (errors, warnings, hints) for a file or directory. ",
                "Returns a list of problems with file path, line/column, severity, and message. ",
                "Use this to understand compiler errors and type errors without running a full build."
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path or directory to get diagnostics for"
                    },
                    "severity": {
                        "type": "string",
                        "enum": ["error", "warning", "info", "hint", "all"],
                        "description": "Minimum severity to include (default: error)"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
    ]
}

pub fn execute_tool(name: &str, input: &Value) -> Result<String, String> {
    match name {
        "bash" => from_value::<BashCommandInput>(input).and_then(run_bash),
        "read_file" => from_value::<ReadFileInput>(input).and_then(run_read_file),
        "write_file" => from_value::<WriteFileInput>(input).and_then(run_write_file),
        "edit_file" => from_value::<EditFileInput>(input).and_then(run_edit_file),
        "glob_search" => from_value::<GlobSearchInputValue>(input).and_then(run_glob_search),
        "grep_search" => from_value::<GrepSearchInput>(input).and_then(run_grep_search),
        "list_directory" => from_value::<ListDirInput>(input).and_then(run_list_directory),
        "web_search" => from_value::<WebSearchInput>(input).and_then(run_web_search),
        "web_fetch" => from_value::<WebFetchInput>(input).and_then(run_web_fetch),
        // E1: Richer diagnostics via LSP integration
        "get_diagnostics" => from_value::<GetDiagnosticsInput>(input).and_then(run_get_diagnostics),
        _ => Err(format!("unsupported tool: {name}")),
    }
}

/// Execute a tool, checking custom tools if the built-in tools don't match.
pub fn execute_tool_with_custom(
    name: &str,
    input: &Value,
    custom_registry: &CustomToolRegistry,
) -> Result<String, String> {
    // Try built-in tools first
    match name {
        "bash" | "read_file" | "write_file" | "edit_file" | "glob_search" | "grep_search"
        | "list_directory" | "web_search" | "web_fetch" | "get_diagnostics" => {
            return execute_tool(name, input);
        }
        _ => {}
    }
    // Try custom tools
    if custom_registry.find(name).is_some() {
        return custom_registry.execute(name, input);
    }
    Err(format!("unsupported tool: {name}"))
}

/// Execute a tool and return an optional diff preview for write/edit operations.
/// This is used by CLI and daemon executors to show/log diffs.
pub fn execute_tool_with_diff(name: &str, input: &Value) -> Result<(String, Option<DiffPreview>), String> {
    match name {
        "write_file" => {
            let parsed: WriteFileInput = from_value(input)?;
            let (output, preview) = write_file(&parsed.path, &parsed.content).map_err(io_to_string)?;
            let json = to_pretty_json(output)?;
            Ok((json, Some(preview)))
        }
        "edit_file" => {
            let parsed: EditFileInput = from_value(input)?;
            let (output, preview) = edit_file(
                &parsed.path,
                &parsed.old_string,
                &parsed.new_string,
                parsed.replace_all.unwrap_or(false),
            ).map_err(io_to_string)?;
            let json = to_pretty_json(output)?;
            Ok((json, Some(preview)))
        }
        _ => {
            let result = execute_tool(name, input)?;
            Ok((result, None))
        }
    }
}

fn from_value<T: for<'de> Deserialize<'de>>(input: &Value) -> Result<T, String> {
    serde_json::from_value(input.clone()).map_err(|error| error.to_string())
}

fn run_bash(input: BashCommandInput) -> Result<String, String> {
    serde_json::to_string_pretty(&execute_bash(input).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn run_read_file(input: ReadFileInput) -> Result<String, String> {
    to_pretty_json(read_file(&input.path, input.offset, input.limit).map_err(io_to_string)?)
}

fn run_write_file(input: WriteFileInput) -> Result<String, String> {
    let (output, _preview) = write_file(&input.path, &input.content).map_err(io_to_string)?;
    to_pretty_json(output)
}

fn run_edit_file(input: EditFileInput) -> Result<String, String> {
    let (output, _preview) = edit_file(
        &input.path,
        &input.old_string,
        &input.new_string,
        input.replace_all.unwrap_or(false),
    )
    .map_err(io_to_string)?;
    to_pretty_json(output)
}

fn run_glob_search(input: GlobSearchInputValue) -> Result<String, String> {
    to_pretty_json(glob_search(&input.pattern, input.path.as_deref()).map_err(io_to_string)?)
}

fn run_grep_search(input: GrepSearchInput) -> Result<String, String> {
    to_pretty_json(grep_search(&input).map_err(io_to_string)?)
}

fn to_pretty_json<T: serde::Serialize>(value: T) -> Result<String, String> {
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

fn io_to_string(error: std::io::Error) -> String {
    error.to_string()
}

#[derive(Debug, Deserialize)]
struct ReadFileInput {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WriteFileInput {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct EditFileInput {
    path: String,
    old_string: String,
    new_string: String,
    replace_all: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GlobSearchInputValue {
    pattern: String,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListDirInput {
    path: Option<String>,
    depth: Option<usize>,
}

fn run_list_directory(input: ListDirInput) -> Result<String, String> {
    to_pretty_json(list_directory(input.path.as_deref(), input.depth).map_err(io_to_string)?)
}

fn run_web_search(input: WebSearchInput) -> Result<String, String> {
    to_pretty_json(web_search(&input)?)
}

fn run_web_fetch(input: WebFetchInput) -> Result<String, String> {
    to_pretty_json(web_fetch(&input)?)
}

// ── E1: Richer diagnostics ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GetDiagnosticsInput {
    path: String,
    #[serde(default)]
    severity: Option<String>,
}

/// Run `get_diagnostics` using available CLI type-checkers.
///
/// Detects the project type from the target path and runs the appropriate
/// checker (cargo check, tsc --noEmit, mypy/pyright, go vet).  Returns a
/// JSON array of diagnostic objects with file/line/severity/message fields.
fn run_get_diagnostics(input: GetDiagnosticsInput) -> Result<String, String> {
    let path = std::path::Path::new(&input.path);
    let min_severity = input.severity.as_deref().unwrap_or("error");

    // Determine workspace root from path
    let workspace = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };

    // Detect language by presence of marker files
    let is_rust   = workspace.join("Cargo.toml").exists() || path.extension().is_some_and(|e| e == "rs");
    let is_ts     = workspace.join("tsconfig.json").exists() || path.extension().is_some_and(|e| e == "ts" || e == "tsx");
    let is_python = path.extension().is_some_and(|e| e == "py")
                    || workspace.join("pyproject.toml").exists()
                    || workspace.join("setup.py").exists();
    let is_go     = workspace.join("go.mod").exists() || path.extension().is_some_and(|e| e == "go");

    let (cmd, args): (&str, Vec<&str>) = if is_rust {
        ("cargo", vec!["check", "--message-format=json", "--quiet"])
    } else if is_ts {
        ("npx", vec!["tsc", "--noEmit", "--pretty", "false"])
    } else if is_python {
        if std::process::Command::new("mypy").arg("--version").output().is_ok() {
            ("mypy", vec![input.path.as_str(), "--no-error-summary", "--show-column-numbers"])
        } else {
            ("python3", vec!["-m", "py_compile", input.path.as_str()])
        }
    } else if is_go {
        ("go", vec!["vet", "./..."])
    } else {
        // Fallback: try a syntax check via the shell
        ("sh", vec!["-n", input.path.as_str()])
    };

    let output = std::process::Command::new(cmd)
        .args(&args)
        .current_dir(&workspace)
        .output()
        .map_err(|e| format!("could not run {cmd}: {e} (is it installed?)"))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Parse output into structured diagnostics
    let diags = if is_rust {
        parse_cargo_diagnostics(&combined, min_severity)
    } else {
        parse_text_diagnostics(&combined, min_severity, &input.path)
    };

    serde_json::to_string_pretty(&diags).map_err(|e| e.to_string())
}

/// Parse `cargo check --message-format=json` output.
fn parse_cargo_diagnostics(output: &str, min_severity: &str) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["reason"].as_str() != Some("compiler-message") { continue; }
        let msg = &v["message"];
        let level = msg["level"].as_str().unwrap_or("note");
        if !severity_passes(level, min_severity) { continue; }
        let text = msg["message"].as_str().unwrap_or("").to_string();
        // Primary span
        if let Some(spans) = msg["spans"].as_array() {
            for span in spans.iter().filter(|s| s["is_primary"].as_bool().unwrap_or(false)) {
                results.push(serde_json::json!({
                    "file": span["file_name"].as_str().unwrap_or(""),
                    "line": span["line_start"].as_u64().unwrap_or(0),
                    "column": span["column_start"].as_u64().unwrap_or(0),
                    "severity": level,
                    "message": text,
                    "source": "cargo",
                }));
            }
        }
    }
    results
}

/// Parse plain-text checker output (tsc, mypy, go vet, etc.).
fn parse_text_diagnostics(output: &str, min_severity: &str, default_file: &str) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    // Common pattern: "file.ext:line:col: error/warning: message"
    let _re_patterns = [
        // TypeScript: src/foo.ts(10,5): error TS2304: ...
        (r"([^:()]+)\((\d+),(\d+)\):\s*(error|warning|info)\s+\w+:\s*(.*)", true),
        // mypy: src/foo.py:10: error: ...
        (r"([^:]+):(\d+):\s*(error|warning|note):\s*(.*)", false),
        // go vet / general: ./pkg/file.go:10:5: message
        (r"(\./[^:]+):(\d+):(\d+):\s*(.*)", false),
    ];

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }

        // Try each pattern manually (no regex crate — keep deps minimal)
        if let Some(d) = try_parse_diagnostic_line(line, default_file, min_severity) {
            results.push(d);
        }
    }
    results
}

fn try_parse_diagnostic_line(line: &str, default_file: &str, min_severity: &str) -> Option<serde_json::Value> {
    // Pattern: <file>:<line>:<col>: <level>: <message>
    // or:      <file>(<line>,<col>): <level> <code>: <message>
    let line = line.replace(['(', ')'], ":");
    let parts: Vec<&str> = line.splitn(6, ':').collect();
    if parts.len() < 3 { return None; }

    let file = parts[0].trim();
    let lineno: u64 = parts[1].trim().parse().unwrap_or(0);
    // parts[2] may be col or level
    let (col, level_idx) = if parts[2].trim().parse::<u64>().is_ok() {
        (parts[2].trim().parse::<u64>().unwrap_or(0), 3)
    } else {
        (0, 2)
    };

    if parts.len() <= level_idx { return None; }
    let level_raw = parts[level_idx].trim().to_lowercase();
    let level = if level_raw.starts_with("error") { "error" }
                else if level_raw.starts_with("warn") { "warning" }
                else if level_raw.starts_with("note") || level_raw.starts_with("info") { "info" }
                else { return None };

    if !severity_passes(level, min_severity) { return None; }

    let msg_parts = &parts[(level_idx + 1)..];
    let message = msg_parts.join(":").trim().to_string();
    if message.is_empty() { return None; }

    Some(serde_json::json!({
        "file": if file.is_empty() { default_file } else { file },
        "line": lineno,
        "column": col,
        "severity": level,
        "message": message,
        "source": "cli",
    }))
}

fn severity_passes(level: &str, min: &str) -> bool {
    let rank = |s: &str| match s {
        "error"   => 0_u8,
        "warning" | "warn" => 1,
        "info" | "note" => 2,
        "hint"    => 3,
        _         => 4,
    };
    min == "all" || rank(level) <= rank(min)
}

#[cfg(test)]
mod tests {
    use super::{execute_tool, mvp_tool_specs};
    use serde_json::json;

    #[test]
    fn exposes_mvp_tools() {
        let names = mvp_tool_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"read_file"));
    }

    #[test]
    fn rejects_unknown_tool_names() {
        let error = execute_tool("nope", &json!({})).expect_err("tool should be rejected");
        assert!(error.contains("unsupported tool"));
    }
}
