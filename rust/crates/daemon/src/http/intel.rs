//! Intelligence handlers: semantic search, fine-tuning, diagnostics, index, dependency graph.

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::state::DaemonState;
use super::{Response, ErrorResponse, urlencoding_decode};

pub(super) fn handle_search(path_full: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let (query, limit) = {
        let qs = path_full.split_once('?').map(|(_, q)| q).unwrap_or("");
        let mut q_val = String::new();
        let mut lim_val: usize = 10;
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "q" | "query" => q_val = urlencoding_decode(v),
                    "limit" | "n" => lim_val = v.parse().unwrap_or(10).min(50),
                    _ => {}
                }
            }
        }
        (q_val, lim_val)
    };
    if query.is_empty() {
        return Response::json(400, &ErrorResponse { error: "missing query param: ?q=".to_string() });
    }
    let s = state.lock().unwrap_or_else(|e| e.into_inner());
    let ws = &s.workspace_root;
    let index = match intelligence::CodebaseIndexer::load_index(ws) {
        Ok(idx) => idx,
        Err(_) => {
            let cfg = intelligence::IndexerConfig::default();
            match intelligence::CodebaseIndexer::build_index(ws, &cfg) {
                Ok(idx) => { let _ = intelligence::CodebaseIndexer::save_index(ws, &idx); idx }
                Err(e) => return Response::json(503, &ErrorResponse { error: format!("codebase not indexed: {e}") }),
            }
        }
    };
    let results: Vec<serde_json::Value> = intelligence::CodebaseIndexer::search(&index, &query, limit)
        .into_iter()
        .map(|entry| serde_json::json!({
            "path": entry.path, "language": entry.language,
            "lines": entry.lines, "exports": entry.exports, "summary": entry.summary,
        }))
        .collect();
    Response::json(200, &serde_json::json!({ "query": query, "results": results }))
}

pub(super) fn handle_finetune_extract(body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { sessions_dir: Option<String> }
    let req: Req = serde_json::from_str(body).unwrap_or(Req { sessions_dir: None });
    let workspace_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let sessions_dir = req.sessions_dir
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root.join(".tachy").join("sessions"));
    let dataset = intelligence::FinetuneDataset::from_sessions(&sessions_dir);
    Response::json(200, &serde_json::json!({
        "entries": dataset.total_pairs,
        "source_sessions": dataset.source_sessions,
        "jsonl": dataset.to_jsonl(),
    }))
}

pub(super) fn handle_finetune_modelfile(body: &str, _state: &Arc<Mutex<DaemonState>>) -> Response {
    #[derive(Deserialize)]
    struct Req { base_model: String, adapter_path: String, system_prompt: Option<String> }
    let req: Req = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => return Response::json(400, &ErrorResponse { error: format!("invalid body: {e}") }),
    };
    let prompt = req.system_prompt.as_deref().unwrap_or("You are a helpful AI coding assistant.");
    let mf = intelligence::generate_modelfile(&req.base_model, &req.adapter_path, prompt);
    Response::json(200, &serde_json::json!({ "modelfile": mf }))
}

pub(super) fn handle_diagnostics(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let file_path = {
        let mut path = String::new();
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "file" || k == "path" { path = urlencoding_decode(v); break; }
            }
        }
        path
    };
    if file_path.is_empty() {
        return Response::json(400, &ErrorResponse { error: "missing ?file= param".to_string() });
    }
    let workspace_root = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let lsp = intelligence::LspManager::new(&workspace_root);
    let diagnostics = lsp.get_diagnostics(&file_path);
    Response::json(200, &serde_json::json!({
        "file": file_path,
        "diagnostics": diagnostics.iter().map(|d| serde_json::json!({
            "file": d.file, "line": d.line, "column": d.column, "message": d.message,
            "severity": match d.severity {
                intelligence::DiagnosticSeverity::Error => "error",
                intelligence::DiagnosticSeverity::Warning => "warning",
                _ => "info",
            },
            "source": d.source,
        })).collect::<Vec<_>>(),
        "count": diagnostics.len(),
    }))
}

pub(super) fn handle_index_build(_body: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let ws_display = ws.display().to_string();
    std::thread::spawn(move || {
        let cfg = intelligence::IndexerConfig::default();
        match intelligence::CodebaseIndexer::build_index(&ws, &cfg) {
            Ok(idx) => eprintln!("[index] build complete: {} files indexed", idx.files.len()),
            Err(e) => eprintln!("[index] build failed: {e}"),
        }
    });
    Response::json(202, &serde_json::json!({
        "status": "building",
        "workspace": ws_display,
        "message": "Index build started in background — poll GET /api/index for status"
    }))
}

pub(super) fn handle_index_status(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    match intelligence::CodebaseIndexer::load_index(&ws) {
        Ok(idx) => Response::json(200, &serde_json::json!({
            "status": "ready", "file_count": idx.files.len(), "workspace": ws.display().to_string()
        })),
        Err(_) => Response::json(200, &serde_json::json!({
            "status": "not_built", "file_count": 0, "workspace": ws.display().to_string()
        })),
    }
}

pub(super) fn handle_dependency_graph(query: &str, state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let graph = intelligence::DependencyGraph::build(&ws);
    let file_param = query
        .split('&')
        .find(|p| p.starts_with("file="))
        .map(|p| urlencoding_decode(&p[5..]));
    if let Some(f) = file_param {
        let deps = graph.transitive_dependents(&f);
        let node = graph.nodes.get(&f);
        return Response::json(200, &serde_json::json!({
            "file": f,
            "direct_imports": node.map(|n| &n.imports).cloned().unwrap_or_default(),
            "imported_by": node.map(|n| &n.imported_by).cloned().unwrap_or_default(),
            "transitive_dependents": deps,
        }));
    }
    Response::json(200, &graph)
}

pub(super) fn handle_monorepo(state: &Arc<Mutex<DaemonState>>) -> Response {
    let ws = state.lock().unwrap_or_else(|e| e.into_inner()).workspace_root.clone();
    let manifest = intelligence::MonorepoManifest::detect(&ws);
    Response::json(200, &manifest)
}
