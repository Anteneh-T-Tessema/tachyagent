//! Output validation — check generated code for basic correctness before applying.
//! Catches common local model mistakes: unbalanced brackets, incomplete code, syntax errors.

use serde::{Deserialize, Serialize};

/// Result of validating generated code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub kind: ValidationErrorKind,
    pub message: String,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationErrorKind {
    UnbalancedBrackets,
    IncompleteCode,
    EmptyOutput,
    SuspiciousPattern,
}

/// Validate generated code for basic correctness.
#[must_use] pub fn validate_code(code: &str, language: &str) -> ValidationResult {
    let mut errors = Vec::new();

    // Check for empty output
    if code.trim().is_empty() {
        errors.push(ValidationError {
            kind: ValidationErrorKind::EmptyOutput,
            message: "generated code is empty".to_string(),
            line: None,
        });
        return ValidationResult { valid: false, errors };
    }

    // Check balanced brackets
    if let Some(err) = check_balanced_brackets(code) {
        errors.push(err);
    }

    // Check for incomplete code patterns
    if let Some(err) = check_incomplete_code(code, language) {
        errors.push(err);
    }

    // Check for suspicious patterns (model artifacts)
    for err in check_suspicious_patterns(code) {
        errors.push(err);
    }

    ValidationResult {
        valid: errors.is_empty(),
        errors,
    }
}

fn check_balanced_brackets(code: &str) -> Option<ValidationError> {
    let mut stack: Vec<(char, usize)> = Vec::new();

    for (line_num, line) in code.lines().enumerate() {
        // Skip strings and comments (simplified)
        let mut in_string = false;
        let mut prev_char = ' ';

        for ch in line.chars() {
            if ch == '"' && prev_char != '\\' {
                in_string = !in_string;
            }
            if in_string {
                prev_char = ch;
                continue;
            }

            match ch {
                '{' | '(' | '[' => stack.push((ch, line_num + 1)),
                '}' => {
                    if stack.last().map(|(c, _)| *c) != Some('{') {
                        return Some(ValidationError {
                            kind: ValidationErrorKind::UnbalancedBrackets,
                            message: format!("unexpected '}}' at line {}", line_num + 1),
                            line: Some(line_num + 1),
                        });
                    }
                    stack.pop();
                }
                ')' => {
                    if stack.last().map(|(c, _)| *c) != Some('(') {
                        return Some(ValidationError {
                            kind: ValidationErrorKind::UnbalancedBrackets,
                            message: format!("unexpected ')' at line {}", line_num + 1),
                            line: Some(line_num + 1),
                        });
                    }
                    stack.pop();
                }
                ']' => {
                    if stack.last().map(|(c, _)| *c) != Some('[') {
                        return Some(ValidationError {
                            kind: ValidationErrorKind::UnbalancedBrackets,
                            message: format!("unexpected ']' at line {}", line_num + 1),
                            line: Some(line_num + 1),
                        });
                    }
                    stack.pop();
                }
                _ => {}
            }
            prev_char = ch;
        }
    }

    if let Some((bracket, line)) = stack.last() {
        return Some(ValidationError {
            kind: ValidationErrorKind::UnbalancedBrackets,
            message: format!("unclosed '{bracket}' opened at line {line}"),
            line: Some(*line),
        });
    }

    None
}

fn check_incomplete_code(code: &str, language: &str) -> Option<ValidationError> {
    let trimmed = code.trim();

    // Check for truncated code (ends mid-statement)
    let last_line = trimmed.lines().last().unwrap_or("");
    let suspicious_endings = ["...", "//", "/*", "TODO", "FIXME", "///"];
    for ending in &suspicious_endings {
        if last_line.trim().ends_with(ending) {
            return Some(ValidationError {
                kind: ValidationErrorKind::IncompleteCode,
                message: format!("code appears truncated (ends with '{ending}')"),
                line: Some(trimmed.lines().count()),
            });
        }
    }

    // Language-specific checks
    match language {
        "rust" => {
            // Rust: check for unclosed fn/impl/struct
            let fn_count = trimmed.matches("fn ").count();
            let open_braces = trimmed.matches('{').count();
            let close_braces = trimmed.matches('}').count();
            if fn_count > 0 && open_braces > close_braces + 1 {
                return Some(ValidationError {
                    kind: ValidationErrorKind::IncompleteCode,
                    message: "function body appears incomplete (more opening than closing braces)".to_string(),
                    line: None,
                });
            }
        }
        "python" => {
            // Python: check for pass-only functions
            if trimmed.ends_with("pass") && trimmed.lines().count() <= 2 {
                return Some(ValidationError {
                    kind: ValidationErrorKind::IncompleteCode,
                    message: "function body is just 'pass' — likely incomplete".to_string(),
                    line: Some(trimmed.lines().count()),
                });
            }
        }
        _ => {}
    }

    None
}

fn check_suspicious_patterns(code: &str) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Model artifacts — common with local models
    let artifacts = [
        ("```", "code block markers in output — model may have included markdown"),
        ("<|im_end|>", "model control token leaked into output"),
        ("<|endoftext|>", "model control token leaked into output"),
        ("[INST]", "model instruction token leaked into output"),
        ("<<SYS>>", "model system token leaked into output"),
    ];

    for (pattern, message) in &artifacts {
        if code.contains(pattern) {
            errors.push(ValidationError {
                kind: ValidationErrorKind::SuspiciousPattern,
                message: (*message).to_string(),
                line: None,
            });
        }
    }

    errors
}

/// Clean model artifacts from generated code.
#[must_use] pub fn clean_code_output(code: &str) -> String {
    let mut cleaned = code.to_string();

    // Remove markdown code block markers
    if cleaned.starts_with("```") {
        if let Some(first_newline) = cleaned.find('\n') {
            cleaned = cleaned[first_newline + 1..].to_string();
        }
    }
    if cleaned.trim_end().ends_with("```") {
        let end = cleaned.rfind("```").unwrap_or(cleaned.len());
        cleaned = cleaned[..end].to_string();
    }

    // Remove model control tokens
    for token in ["<|im_end|>", "<|endoftext|>", "[INST]", "[/INST]", "<<SYS>>", "<</SYS>>"] {
        cleaned = cleaned.replace(token, "");
    }

    cleaned.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_balanced_brackets() {
        let result = validate_code("fn main() { println!(\"hi\"); }", "rust");
        assert!(result.valid);

        let result = validate_code("fn main() { println!(\"hi\"); ", "rust");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.kind == ValidationErrorKind::UnbalancedBrackets));
    }

    #[test]
    fn detects_empty_output() {
        let result = validate_code("", "rust");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.kind == ValidationErrorKind::EmptyOutput));
    }

    #[test]
    fn detects_truncated_code() {
        let result = validate_code("fn main() {\n    // TODO", "rust");
        assert!(!result.valid);
    }

    #[test]
    fn detects_model_artifacts() {
        let result = validate_code("fn main() {}\n<|im_end|>", "rust");
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.kind == ValidationErrorKind::SuspiciousPattern));
    }

    #[test]
    fn cleans_code_output() {
        let dirty = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let clean = clean_code_output(dirty);
        assert_eq!(clean, "fn main() {\n    println!(\"hi\");\n}");

        let with_tokens = "fn main() {}<|im_end|>";
        let clean = clean_code_output(with_tokens);
        assert_eq!(clean, "fn main() {}");
    }

    #[test]
    fn handles_strings_with_brackets() {
        // Brackets inside strings should not count
        let code = r#"let s = "hello {world}";"#;
        let result = validate_code(code, "rust");
        assert!(result.valid);
    }

    #[test]
    fn valid_python_code_passes() {
        let code = "def hello():\n    print('world')\n    return True";
        let result = validate_code(code, "python");
        assert!(result.valid);
    }
}
