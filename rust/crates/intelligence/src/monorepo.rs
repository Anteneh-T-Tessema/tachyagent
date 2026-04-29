//! Monorepo workspace detection.
//!
//! Identifies the workspace layout for common multi-project toolchains:
//!
//! | Toolchain   | Detection signal                              |
//! |-------------|-----------------------------------------------|
//! | Cargo       | `Cargo.toml` containing `[workspace]`         |
//! | npm/yarn/pnpm | `package.json` with `workspaces` key        |
//! | Turborepo   | `turbo.json` + npm workspaces                 |
//! | Nx          | `nx.json`                                     |
//! | Go          | ≥ 2 `go.mod` files at depth ≤ 4              |
//! | Python      | ≥ 2 `pyproject.toml` files at depth ≤ 4      |

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of monorepo toolchain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MonorepoKind {
    Cargo,
    Npm,
    Pnpm,
    Yarn,
    Turborepo,
    Nx,
    GoModules,
    Python,
    /// No recognized monorepo structure found.
    None,
}

/// A single sub-project (member) within the monorepo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    /// Workspace-relative path to the member root directory.
    pub path: String,
    /// Name of the member (from package manifest or directory name).
    pub name: String,
    /// Primary programming language.
    pub language: String,
}

/// The full monorepo manifest for a workspace root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonorepoManifest {
    /// Detected toolchain.
    pub kind: MonorepoKind,
    /// Absolute path of the workspace root.
    pub root: String,
    /// All discovered sub-projects.
    pub members: Vec<WorkspaceMember>,
    /// `true` when more than one member was found.
    pub is_monorepo: bool,
}

impl MonorepoManifest {
    /// Detect the workspace layout for `root`, trying each toolchain in
    /// priority order and returning the first match.
    #[must_use]
    pub fn detect(root: &Path) -> Self {
        if let Some(m) = detect_cargo(root) {
            return m;
        }
        if let Some(m) = detect_turborepo(root) {
            return m;
        }
        if let Some(m) = detect_nx(root) {
            return m;
        }
        if let Some(m) = detect_npm(root) {
            return m;
        }
        if let Some(m) = detect_go_modules(root) {
            return m;
        }
        if let Some(m) = detect_python(root) {
            return m;
        }
        MonorepoManifest {
            kind: MonorepoKind::None,
            root: root.to_string_lossy().to_string(),
            members: Vec::new(),
            is_monorepo: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Toolchain detectors
// ---------------------------------------------------------------------------

fn detect_cargo(root: &Path) -> Option<MonorepoManifest> {
    let cargo_toml_path = root.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&cargo_toml_path).ok()?;
    if !content.contains("[workspace]") {
        return None;
    }
    let members = parse_cargo_workspace_members(&content);
    Some(MonorepoManifest {
        is_monorepo: members.len() > 1,
        kind: MonorepoKind::Cargo,
        root: root.to_string_lossy().to_string(),
        members,
    })
}

fn parse_cargo_workspace_members(content: &str) -> Vec<WorkspaceMember> {
    let Some(members_idx) = content.find("members") else {
        return Vec::new();
    };
    let after = &content[members_idx..];
    extract_quoted_strings(after)
        .into_iter()
        .map(|p| {
            // p may be a glob like "crates/*" — strip the glob suffix for the display name
            let base = p.trim_end_matches("/*").trim_end_matches("/**");
            let name = Path::new(base)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(base)
                .to_string();
            WorkspaceMember {
                path: base.to_string(),
                name,
                language: "rust".to_string(),
            }
        })
        .collect()
}

fn detect_npm(root: &Path) -> Option<MonorepoManifest> {
    let pkg_path = root.join("package.json");
    if !pkg_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&pkg_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let workspaces = json.get("workspaces")?;

    let patterns: Vec<String> = match workspaces {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::Object(obj) => obj
            .get("packages")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        _ => return None,
    };

    let kind = if root.join("pnpm-workspace.yaml").exists() {
        MonorepoKind::Pnpm
    } else if root.join("yarn.lock").exists() {
        MonorepoKind::Yarn
    } else {
        MonorepoKind::Npm
    };

    let members = expand_npm_workspace_patterns(root, &patterns);
    Some(MonorepoManifest {
        is_monorepo: members.len() > 1,
        kind,
        root: root.to_string_lossy().to_string(),
        members,
    })
}

fn expand_npm_workspace_patterns(root: &Path, patterns: &[String]) -> Vec<WorkspaceMember> {
    let mut members = Vec::new();
    for pattern in patterns {
        let base_str = pattern.trim_end_matches("/*").trim_end_matches("/**");
        let base_dir = root.join(base_str);
        if !base_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&base_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let pkg_json = entry.path().join("package.json");
            if !pkg_json.exists() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = std::fs::read_to_string(&pkg_json)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|j| j["name"].as_str().map(str::to_string))
                .unwrap_or_else(|| rel.clone());
            members.push(WorkspaceMember {
                path: rel,
                name,
                language: "javascript".to_string(),
            });
        }
    }
    members
}

fn detect_turborepo(root: &Path) -> Option<MonorepoManifest> {
    if !root.join("turbo.json").exists() {
        return None;
    }
    // Turborepo always wraps npm workspaces
    let mut m = detect_npm(root)?;
    m.kind = MonorepoKind::Turborepo;
    Some(m)
}

fn detect_nx(root: &Path) -> Option<MonorepoManifest> {
    if !root.join("nx.json").exists() {
        return None;
    }
    let base_dir = if root.join("apps").is_dir() {
        root.join("apps")
    } else if root.join("packages").is_dir() {
        root.join("packages")
    } else {
        root.to_path_buf()
    };

    let mut members = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let rel = entry
                    .path()
                    .strip_prefix(root)
                    .map(|r| r.to_string_lossy().to_string())
                    .unwrap_or_default();
                let name = entry.file_name().to_string_lossy().to_string();
                members.push(WorkspaceMember {
                    path: rel,
                    name,
                    language: "typescript".to_string(),
                });
            }
        }
    }
    Some(MonorepoManifest {
        kind: MonorepoKind::Nx,
        root: root.to_string_lossy().to_string(),
        is_monorepo: members.len() > 1,
        members,
    })
}

fn detect_go_modules(root: &Path) -> Option<MonorepoManifest> {
    let mut go_mods: Vec<PathBuf> = Vec::new();
    find_go_mod(root, root, &mut go_mods, 0);
    if go_mods.len() < 2 {
        return None;
    }
    let members: Vec<WorkspaceMember> = go_mods
        .iter()
        .map(|p| {
            let parent = p.parent().unwrap_or(root);
            let rel = parent
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = parent
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            WorkspaceMember {
                path: rel,
                name,
                language: "go".to_string(),
            }
        })
        .collect();
    Some(MonorepoManifest {
        kind: MonorepoKind::GoModules,
        root: root.to_string_lossy().to_string(),
        is_monorepo: members.len() > 1,
        members,
    })
}

fn find_go_mod(root: &Path, dir: &Path, mods: &mut Vec<PathBuf>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && entry.file_name() == "go.mod" {
            mods.push(path);
        } else if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !matches!(name.as_str(), ".git" | "vendor" | "node_modules") {
                find_go_mod(root, &path, mods, depth + 1);
            }
        }
    }
}

fn detect_python(root: &Path) -> Option<MonorepoManifest> {
    let mut pyprojects: Vec<PathBuf> = Vec::new();
    find_pyproject(root, root, &mut pyprojects, 0);
    if pyprojects.len() < 2 {
        return None;
    }
    let members: Vec<WorkspaceMember> = pyprojects
        .iter()
        .map(|p| {
            let parent = p.parent().unwrap_or(root);
            let rel = parent
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = parent
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            WorkspaceMember {
                path: rel,
                name,
                language: "python".to_string(),
            }
        })
        .collect();
    Some(MonorepoManifest {
        kind: MonorepoKind::Python,
        root: root.to_string_lossy().to_string(),
        is_monorepo: members.len() > 1,
        members,
    })
}

fn find_pyproject(root: &Path, dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && entry.file_name() == "pyproject.toml" {
            out.push(path);
        } else if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !matches!(
                name.as_str(),
                ".git" | "node_modules" | "__pycache__" | ".venv" | "dist" | "build"
            ) {
                find_pyproject(root, &path, out, depth + 1);
            }
        }
    }
}

/// Extract all quoted strings from a TOML snippet (used for members arrays).
/// Stops at the first `]` that closes the array.
fn extract_quoted_strings(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let quote = c;
            let mut token = String::new();
            for ch in chars.by_ref() {
                if ch == quote {
                    break;
                }
                token.push(ch);
            }
            if !token.is_empty() {
                result.push(token);
            }
        }
        if c == ']' {
            break;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_test_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tachy-monorepo-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_cargo_workspace() {
        let dir = make_test_dir("cargo");
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/foo\", \"crates/bar\"]\n",
        )
        .unwrap();
        let m = MonorepoManifest::detect(&dir);
        assert_eq!(m.kind, MonorepoKind::Cargo);
        assert_eq!(m.members.len(), 2);
        assert!(m.is_monorepo);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn single_package_cargo_not_monorepo() {
        let dir = make_test_dir("single-cargo");
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();
        let m = MonorepoManifest::detect(&dir);
        assert_eq!(m.kind, MonorepoKind::None);
        assert!(!m.is_monorepo);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_go_multi_module() {
        let dir = make_test_dir("go-multi");
        fs::create_dir_all(dir.join("svc-a")).unwrap();
        fs::create_dir_all(dir.join("svc-b")).unwrap();
        fs::write(
            dir.join("svc-a/go.mod"),
            "module github.com/org/a\ngo 1.21\n",
        )
        .unwrap();
        fs::write(
            dir.join("svc-b/go.mod"),
            "module github.com/org/b\ngo 1.21\n",
        )
        .unwrap();
        let m = MonorepoManifest::detect(&dir);
        assert_eq!(m.kind, MonorepoKind::GoModules);
        assert!(m.is_monorepo);
        assert_eq!(m.members.len(), 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_python_multi_module() {
        let dir = make_test_dir("py-multi");
        fs::create_dir_all(dir.join("pkg-a")).unwrap();
        fs::create_dir_all(dir.join("pkg-b")).unwrap();
        fs::write(
            dir.join("pkg-a/pyproject.toml"),
            "[project]\nname = \"pkg-a\"\n",
        )
        .unwrap();
        fs::write(
            dir.join("pkg-b/pyproject.toml"),
            "[project]\nname = \"pkg-b\"\n",
        )
        .unwrap();
        let m = MonorepoManifest::detect(&dir);
        assert_eq!(m.kind, MonorepoKind::Python);
        assert!(m.is_monorepo);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn unknown_returns_none_kind() {
        let dir = make_test_dir("unknown");
        let m = MonorepoManifest::detect(&dir);
        assert_eq!(m.kind, MonorepoKind::None);
        assert!(!m.is_monorepo);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cargo_glob_members_are_stripped() {
        let members = parse_cargo_workspace_members("[workspace]\nmembers = [\"crates/*\"]\n");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].path, "crates");
        assert_eq!(members[0].name, "crates");
    }
}
