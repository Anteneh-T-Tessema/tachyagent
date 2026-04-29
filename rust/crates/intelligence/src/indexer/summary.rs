//! Summary extraction: exports, doc comments, and rich structural descriptions.

pub(crate) fn extract_summary(path: &str, content: &str, language: &str) -> (Vec<String>, String) {
    let mut exports = Vec::new();

    for line in content.lines().take(500) {
        let trimmed = line.trim();
        match language {
            "rust" => {
                if let Some(name) = extract_rust_export(trimmed) {
                    exports.push(name);
                }
            }
            "python" => {
                if let Some(name) = extract_python_export(line) {
                    exports.push(name);
                }
            }
            "typescript" | "javascript" => {
                if let Some(name) = extract_ts_export(trimmed) {
                    exports.push(name);
                }
            }
            "go" => {
                if let Some(name) = extract_go_export(trimmed) {
                    exports.push(name);
                }
            }
            _ => {}
        }
        if exports.len() >= 20 {
            break;
        }
    }

    let summary = build_rich_summary(path, content, language, &exports);
    (exports, summary)
}

/// Build a meaningful summary from doc comments, module descriptions, and
/// path semantics — never just an import line.
fn build_rich_summary(path: &str, content: &str, language: &str, exports: &[String]) -> String {
    // 1. Try to extract the module-level doc comment (most informative)
    if let Some(doc) = extract_module_doc(content, language) {
        let module_hint = path_to_module_hint(path);
        if module_hint.is_empty() {
            return truncate_summary(&doc, 160);
        }
        return truncate_summary(&format!("{module_hint}: {doc}"), 160);
    }

    // 2. No doc comment — build a structural description from path + exports
    let module_hint = path_to_module_hint(path);
    if !exports.is_empty() {
        let top: Vec<&str> = exports.iter().take(6).map(String::as_str).collect();
        let export_list = top.join(", ");
        if module_hint.is_empty() {
            return truncate_summary(&format!("defines {export_list}"), 160);
        }
        return truncate_summary(&format!("{module_hint}: defines {export_list}"), 160);
    }

    // 3. Fall back to module hint alone
    if !module_hint.is_empty() {
        return module_hint;
    }

    // 4. Last resort: first non-import, non-comment content line
    let fallback = content
        .lines()
        .find(|line| {
            let t = line.trim();
            !t.is_empty()
                && !t.starts_with("//")
                && !t.starts_with('#')
                && !t.starts_with("use ")
                && !t.starts_with("import ")
                && !t.starts_with("from ")
                && !t.starts_with("package ")
                && !t.starts_with("mod ")
                && !t.starts_with("pub mod ")
        })
        .unwrap_or("")
        .trim();

    truncate_summary(fallback, 120)
}

/// Extract the module-level doc comment for the language.
/// Returns the first meaningful doc block as a single cleaned string.
fn extract_module_doc(content: &str, language: &str) -> Option<String> {
    match language {
        "rust" => {
            // //! inner doc comments at the top of the file
            let doc_lines: Vec<&str> = content
                .lines()
                .take(30)
                .filter(|l| l.trim_start().starts_with("//!"))
                .map(|l| l.trim_start().trim_start_matches("//!").trim())
                .filter(|l| !l.is_empty())
                .collect();
            if doc_lines.is_empty() {
                // Also try /// on the first pub item
                let triple_lines: Vec<&str> = content
                    .lines()
                    .take(60)
                    .filter(|l| l.trim_start().starts_with("/// "))
                    .map(|l| l.trim_start().trim_start_matches("/// ").trim())
                    .filter(|l| !l.is_empty())
                    .take(3)
                    .collect();
                if triple_lines.is_empty() {
                    return None;
                }
                return Some(triple_lines.join(" "));
            }
            Some(doc_lines.join(" "))
        }
        "python" => {
            // Module docstring: first string literal after optional shebang/encoding
            let mut in_doc = false;
            let mut doc_lines = Vec::new();
            let mut delimiter = "";
            for line in content.lines().take(40) {
                let t = line.trim();
                if in_doc {
                    if t.contains(delimiter) {
                        let before = t.split(delimiter).next().unwrap_or("").trim();
                        if !before.is_empty() {
                            doc_lines.push(before);
                        }
                        break;
                    }
                    if !t.is_empty() {
                        doc_lines.push(t);
                    }
                    if doc_lines.len() >= 4 {
                        break;
                    }
                } else if t.starts_with("\"\"\"") || t.starts_with("'''") {
                    delimiter = if t.starts_with("\"\"\"") {
                        "\"\"\""
                    } else {
                        "'''"
                    };
                    let rest = t.trim_start_matches(delimiter);
                    // Single-line docstring
                    if let Some(end) = rest.find(delimiter) {
                        let single = rest[..end].trim();
                        if !single.is_empty() {
                            return Some(single.to_string());
                        }
                        return None;
                    }
                    if !rest.trim().is_empty() {
                        doc_lines.push(rest.trim());
                    }
                    in_doc = true;
                } else if t.starts_with('#')
                    || t.is_empty()
                    || t.starts_with("import ")
                    || t.starts_with("from ")
                {
                    // skip imports/comments
                } else {
                    break; // hit code, no module docstring
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join(" "))
            }
        }
        "typescript" | "javascript" => {
            // /** ... */ JSDoc at top of file
            let mut in_block = false;
            let mut doc_lines = Vec::new();
            for line in content.lines().take(30) {
                let t = line.trim();
                if in_block {
                    if t.contains("*/") {
                        let before = t
                            .split("*/")
                            .next()
                            .unwrap_or("")
                            .trim_start_matches('*')
                            .trim();
                        if !before.is_empty() {
                            doc_lines.push(before.to_string());
                        }
                        break;
                    }
                    let rest = t.trim_start_matches('*').trim();
                    if !rest.is_empty() {
                        doc_lines.push(rest.to_string());
                    }
                    if doc_lines.len() >= 4 {
                        break;
                    }
                } else if t.starts_with("/**") {
                    in_block = true;
                    let rest = t.trim_start_matches("/**").trim_end_matches("*/").trim();
                    if !rest.is_empty() {
                        doc_lines.push(rest.to_string());
                    }
                } else if t.starts_with("//") {
                    let rest = t.trim_start_matches("//").trim();
                    if !rest.is_empty() {
                        doc_lines.push(rest.to_string());
                    }
                    if doc_lines.len() >= 3 {
                        break;
                    }
                } else if !t.is_empty() && !t.starts_with("import") && !t.starts_with("'use") {
                    break;
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join(" "))
            }
        }
        "go" => {
            // Package comment: lines starting with // before `package`
            let mut doc_lines = Vec::new();
            for line in content.lines().take(30) {
                let t = line.trim();
                if t.starts_with("package ") {
                    break;
                }
                if let Some(rest) = t.strip_prefix("//") {
                    let rest = rest.trim();
                    if !rest.is_empty() {
                        doc_lines.push(rest);
                    }
                } else if !t.is_empty() {
                    doc_lines.clear(); // reset on non-comment before package
                }
            }
            if doc_lines.is_empty() {
                None
            } else {
                Some(doc_lines.join(" "))
            }
        }
        _ => None,
    }
}

/// Convert a file path into readable semantic tokens.
/// "audit/src/security.rs" → "audit security"
/// "daemon/src/http.rs"    → "daemon http"
fn path_to_module_hint(path: &str) -> String {
    let without_ext = path
        .rsplit('.')
        .nth(1)
        .map_or(path, |_| path.rsplit_once('.').map_or(path, |(l, _)| l));
    let parts: Vec<&str> = without_ext
        .split('/')
        .filter(|p| !matches!(*p, "src" | "lib" | "mod" | "index" | "main" | "." | ".."))
        .collect();
    parts.join(" ")
}

fn truncate_summary(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        s.to_string()
    } else {
        // find a char boundary
        let boundary = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i < max.saturating_sub(1))
            .last()
            .unwrap_or(0);
        format!("{}…", &s[..boundary])
    }
}

pub(crate) fn extract_rust_export(line: &str) -> Option<String> {
    for prefix in [
        "pub fn ",
        "pub struct ",
        "pub enum ",
        "pub trait ",
        "pub type ",
        "pub mod ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()?;
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_python_export(line: &str) -> Option<String> {
    if !line.starts_with(' ') && !line.starts_with('\t') {
        for prefix in ["def ", "class "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                let name = rest
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()?;
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

pub(crate) fn extract_ts_export(line: &str) -> Option<String> {
    for prefix in [
        "export function ",
        "export class ",
        "export const ",
        "export interface ",
        "export type ",
        "export default function ",
        "export default class ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()?;
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_go_export(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("func ") {
        // Skip methods: func (r *Receiver) Name()
        let rest = if rest.starts_with('(') {
            rest.split(')').nth(1)?.trim()
        } else {
            rest
        };
        let name = rest
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()?;
        if !name.is_empty() && name.chars().next()?.is_uppercase() {
            return Some(name.to_string());
        }
    }
    if let Some(rest) = line.strip_prefix("type ") {
        let name = rest.split_whitespace().next()?;
        if name.chars().next()?.is_uppercase() {
            return Some(name.to_string());
        }
    }
    None
}
