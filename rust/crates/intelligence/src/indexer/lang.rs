//! Language detection, test-command detection, and file-filtering helpers.

use std::path::Path;

/// Detect the programming language from a file path extension.
pub fn detect_language(path: &str) -> &str {
    match path.rsplit('.').next() {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts" | "tsx") => "typescript",
        Some("js" | "jsx") => "javascript",
        Some("go") => "go",
        Some("java") => "java",
        Some("rb") => "ruby",
        Some("c" | "h") => "c",
        Some("cpp" | "cc" | "hpp") => "cpp",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("yaml" | "yml") => "yaml",
        Some("md") => "markdown",
        Some("sh" | "bash") => "shell",
        Some("sql") => "sql",
        Some("html" | "htm") => "html",
        Some("css" | "scss") => "css",
        Some("swift") => "swift",
        Some("kt" | "kts") => "kotlin",
        _ => "unknown",
    }
}

/// Detect the test command for the workspace (based on marker files).
pub fn detect_test_command(workspace_root: &Path) -> Option<String> {
    if workspace_root.join("Cargo.toml").exists() {
        Some("cargo test".to_string())
    } else if workspace_root.join("package.json").exists() {
        Some("npm test".to_string())
    } else if workspace_root.join("pyproject.toml").exists()
        || workspace_root.join("setup.py").exists()
    {
        Some("pytest".to_string())
    } else if workspace_root.join("go.mod").exists() {
        Some("go test ./...".to_string())
    } else if workspace_root.join("Makefile").exists() {
        Some("make test".to_string())
    } else {
        None
    }
}

pub(crate) fn detect_build_system(workspace_root: &Path) -> Option<String> {
    if workspace_root.join("Cargo.toml").exists() {
        Some("cargo".to_string())
    } else if workspace_root.join("package.json").exists() {
        Some("npm".to_string())
    } else if workspace_root.join("go.mod").exists() {
        Some("go".to_string())
    } else if workspace_root.join("pyproject.toml").exists() {
        Some("python".to_string())
    } else {
        None
    }
}

pub(crate) fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".tachy" | "node_modules" | "target" | "__pycache__"
            | ".venv" | "vendor" | ".next" | "dist" | "build"
            | ".idea" | ".vscode" | ".DS_Store"
    )
}

pub(crate) fn is_binary_extension(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "svg"
            | "wasm" | "o" | "so" | "dylib" | "dll" | "exe" | "a"
            | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z"
            | "pdf" | "doc" | "docx" | "xls" | "xlsx"
            | "mp3" | "mp4" | "avi" | "mov" | "wav"
            | "ttf" | "otf" | "woff" | "woff2" | "eot"
            | "lock" | "bin"
    )
}
