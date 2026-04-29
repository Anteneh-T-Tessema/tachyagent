//! Dependency / call-graph builder.
//!
//! Scans every source file under a workspace root for import/use/mod
//! statements and builds a directed dependency graph:
//!   edge A → B  means "file A depends on file B".
//!
//! Supported languages: Rust, TypeScript, JavaScript, Python, Go.
//! The analysis is text-based (no full AST) — fast and allocation-light.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single node in the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Workspace-relative path of the file.
    pub path: String,
    /// Files this file imports (forward edges, workspace-relative paths).
    pub imports: Vec<String>,
    /// Files that import this file (reverse edges).
    pub imported_by: Vec<String>,
    /// Programming language detected from extension.
    pub language: String,
}

/// The full dependency graph for a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependencyGraph {
    /// Nodes keyed by workspace-relative path.
    pub nodes: BTreeMap<String, GraphNode>,
    /// Total forward-edge count.
    pub edge_count: usize,
    /// Unix timestamp when the graph was built.
    pub built_at: u64,
    /// Absolute path of the workspace root.
    pub workspace_root: String,
}

impl DependencyGraph {
    /// Build a full dependency graph by scanning all source files under `root`.
    #[must_use]
    pub fn build(root: &Path) -> Self {
        let built_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
        collect_files(root, root, &mut nodes);

        // Second pass: build imported_by (reverse edges) from forward edges
        let forward: Vec<(String, Vec<String>)> = nodes
            .iter()
            .map(|(k, v)| (k.clone(), v.imports.clone()))
            .collect();

        for (importer, deps) in &forward {
            for dep in deps {
                if let Some(node) = nodes.get_mut(dep) {
                    if !node.imported_by.contains(importer) {
                        node.imported_by.push(importer.clone());
                    }
                }
            }
        }

        let edge_count: usize = nodes.values().map(|n| n.imports.len()).sum();

        DependencyGraph {
            nodes,
            edge_count,
            built_at,
            workspace_root: root.to_string_lossy().to_string(),
        }
    }

    /// Return all transitive dependents of `file` via BFS over reverse edges.
    ///
    /// These are files that could be affected if `file` changes.
    #[must_use]
    pub fn transitive_dependents(&self, file: &str) -> Vec<String> {
        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(file.to_string());
        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(node) = self.nodes.get(&current) {
                for dep in &node.imported_by {
                    queue.push_back(dep.clone());
                }
            }
        }
        visited.remove(file);
        visited.into_iter().collect()
    }

    /// Return the direct imports of `file`.
    #[must_use]
    pub fn direct_imports(&self, file: &str) -> Vec<String> {
        self.nodes
            .get(file)
            .map(|n| n.imports.clone())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// File collection
// ---------------------------------------------------------------------------

fn collect_files(root: &Path, dir: &Path, nodes: &mut BTreeMap<String, GraphNode>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if is_ignored_dir(&name) {
                continue;
            }
            collect_files(root, &path, nodes);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let Some(language) = lang_for_ext(ext) else {
                continue;
            };

            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let imports = extract_imports(&content, language, root, &path);
            nodes.insert(
                rel.clone(),
                GraphNode {
                    path: rel,
                    imports,
                    imported_by: Vec::new(),
                    language: language.to_string(),
                },
            );
        }
    }
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "target"
            | "node_modules"
            | ".git"
            | "dist"
            | "build"
            | ".tachy"
            | "__pycache__"
            | ".venv"
            | "vendor"
    )
}

fn lang_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" => Some("kotlin"),
        "swift" => Some("swift"),
        "cpp" | "cc" | "cxx" => Some("cpp"),
        "c" | "h" => Some("c"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Import extraction (text-based, per language)
// ---------------------------------------------------------------------------

fn extract_imports(content: &str, language: &str, root: &Path, current_file: &Path) -> Vec<String> {
    let mut imports = Vec::new();
    let current_dir = current_file.parent().unwrap_or(root);

    for line in content.lines() {
        let line = line.trim();
        match language {
            "rust" => {
                // `mod foo;` or `pub mod foo;`
                if let Some(rest) = line
                    .strip_prefix("mod ")
                    .or_else(|| line.strip_prefix("pub mod "))
                {
                    let mod_name = rest.trim_end_matches(';').trim();
                    if !mod_name.contains('{') && !mod_name.is_empty() {
                        try_resolve_rust_module(mod_name, current_dir, root, &mut imports);
                    }
                }
                // `use crate::foo` or `use super::foo`
                if let Some(rest) = line
                    .strip_prefix("use ")
                    .or_else(|| line.strip_prefix("pub use "))
                {
                    let path = rest.trim_end_matches(';');
                    if path.starts_with("crate::") || path.starts_with("super::") {
                        let skip = if path.starts_with("crate::") {
                            "crate::".len()
                        } else {
                            "super::".len()
                        };
                        let submod = path[skip..].split("::").next().unwrap_or("").trim();
                        if !submod.is_empty() {
                            try_resolve_rust_module(submod, current_dir, root, &mut imports);
                        }
                    }
                }
            }
            "typescript" | "javascript" => {
                if let Some(p) = extract_js_import_path(line) {
                    if p.starts_with('.') {
                        if let Some(resolved) = resolve_relative(
                            p,
                            current_dir,
                            root,
                            &["ts", "tsx", "js", "jsx", "mjs"],
                        ) {
                            imports.push(resolved);
                        }
                    }
                }
            }
            "python" => {
                // `from .foo import bar`
                if let Some(rest) = line.strip_prefix("from .") {
                    let mod_part = rest
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches('.');
                    if !mod_part.is_empty() {
                        let path_guess = current_dir
                            .join(mod_part.replace('.', "/"))
                            .with_extension("py");
                        if path_guess.exists() {
                            if let Ok(r) = path_guess.strip_prefix(root) {
                                imports.push(r.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
            _ => {} // other languages: no import resolution yet
        }
    }

    imports.sort();
    imports.dedup();
    imports
}

fn try_resolve_rust_module(
    mod_name: &str,
    current_dir: &Path,
    root: &Path,
    imports: &mut Vec<String>,
) {
    let candidates = [
        current_dir.join(format!("{mod_name}.rs")),
        current_dir.join(mod_name).join("mod.rs"),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            if let Ok(r) = candidate.strip_prefix(root) {
                imports.push(r.to_string_lossy().to_string());
                return;
            }
        }
    }
}

/// Extract the path string from a JS/TS `import ... from 'path'` or
/// `require('path')` statement.
fn extract_js_import_path(line: &str) -> Option<&str> {
    // import ... from 'path' / "path"
    if let Some(from_idx) = line.rfind(" from ") {
        let rest = line[from_idx + 6..].trim();
        if let Some(quote) = rest.chars().next() {
            if matches!(quote, '\'' | '"' | '`') {
                let inner = &rest[1..];
                if let Some(end) = inner.find(quote) {
                    return Some(&inner[..end]);
                }
            }
        }
    }
    // require('path') / require("path")
    if let Some(req_idx) = line.find("require(") {
        let rest = &line[req_idx + 8..];
        if let Some(quote) = rest.chars().next() {
            if matches!(quote, '\'' | '"') {
                let inner = &rest[1..];
                if let Some(end) = inner.find(quote) {
                    return Some(&inner[..end]);
                }
            }
        }
    }
    None
}

fn resolve_relative(
    import_path: &str,
    current_dir: &Path,
    root: &Path,
    exts: &[&str],
) -> Option<String> {
    let base = current_dir.join(import_path);

    // Try exact path
    if base.exists() && base.is_file() {
        if let Ok(r) = base.strip_prefix(root) {
            return Some(r.to_string_lossy().to_string());
        }
    }
    // Try appending each extension
    for ext in exts {
        let candidate = PathBuf::from(format!("{}.{ext}", base.to_string_lossy()));
        if candidate.exists() {
            if let Ok(r) = candidate.strip_prefix(root) {
                return Some(r.to_string_lossy().to_string());
            }
        }
    }
    // Try index file
    for ext in exts {
        let candidate = base.join(format!("index.{ext}"));
        if candidate.exists() {
            if let Ok(r) = candidate.strip_prefix(root) {
                return Some(r.to_string_lossy().to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tachy-depgraph-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_dir_builds_empty_graph() {
        let dir = make_test_dir("empty");
        let g = DependencyGraph::build(&dir);
        assert_eq!(g.nodes.len(), 0);
        assert_eq!(g.edge_count, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_rust_mod_declarations() {
        let dir = make_test_dir("rustmod");
        std::fs::write(dir.join("main.rs"), "mod foo;\nmod bar;\n").unwrap();
        std::fs::write(dir.join("foo.rs"), "// foo").unwrap();
        std::fs::write(dir.join("bar.rs"), "// bar").unwrap();
        let g = DependencyGraph::build(&dir);
        let main = g.nodes.get("main.rs").expect("main.rs in graph");
        assert!(main.imports.contains(&"foo.rs".to_string()));
        assert!(main.imports.contains(&"bar.rs".to_string()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn builds_reverse_imported_by_edges() {
        let dir = make_test_dir("revdge");
        std::fs::write(dir.join("main.rs"), "mod lib;\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "// lib").unwrap();
        let g = DependencyGraph::build(&dir);
        let lib = g.nodes.get("lib.rs").expect("lib.rs in graph");
        assert!(lib.imported_by.contains(&"main.rs".to_string()));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transitive_dependents_follows_chain() {
        let dir = make_test_dir("transdep");
        std::fs::write(dir.join("a.rs"), "mod b;\n").unwrap();
        std::fs::write(dir.join("b.rs"), "mod c;\n").unwrap();
        std::fs::write(dir.join("c.rs"), "// leaf").unwrap();
        let g = DependencyGraph::build(&dir);
        let deps = g.transitive_dependents("c.rs");
        assert!(deps.contains(&"b.rs".to_string()), "b depends on c");
        assert!(
            deps.contains(&"a.rs".to_string()),
            "a transitively depends on c"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn direct_imports_returns_first_level_only() {
        let dir = make_test_dir("directimports");
        std::fs::write(dir.join("a.rs"), "mod b;\n").unwrap();
        std::fs::write(dir.join("b.rs"), "mod c;\n").unwrap();
        std::fs::write(dir.join("c.rs"), "// leaf").unwrap();
        let g = DependencyGraph::build(&dir);
        let di = g.direct_imports("a.rs");
        assert_eq!(di, vec!["b.rs".to_string()]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn extract_js_import_path_double_quoted() {
        assert_eq!(
            extract_js_import_path(r#"import foo from "./utils""#),
            Some("./utils")
        );
    }

    #[test]
    fn extract_js_import_path_single_quoted() {
        assert_eq!(
            extract_js_import_path("import bar from '../lib'"),
            Some("../lib")
        );
    }

    #[test]
    fn extract_js_require_path() {
        assert_eq!(
            extract_js_import_path("const x = require('./config')"),
            Some("./config")
        );
    }

    #[test]
    fn edge_count_matches_forward_edges() {
        let dir = make_test_dir("edgecount");
        std::fs::write(dir.join("a.rs"), "mod b;\nmod c;\n").unwrap();
        std::fs::write(dir.join("b.rs"), "mod c;\n").unwrap();
        std::fs::write(dir.join("c.rs"), "// leaf").unwrap();
        let g = DependencyGraph::build(&dir);
        // a→b, a→c, b→c = 3
        assert_eq!(g.edge_count, 3);
        std::fs::remove_dir_all(dir).ok();
    }
}
