use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Per-language tool commands.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LanguageTools {
    pub format: Option<String>,
    pub lint: Option<String>,
    pub typecheck: Option<String>,
}

/// Toolchain configuration: language → tool commands.
pub type ToolchainConfig = HashMap<String, LanguageTools>;

/// Result of running a single tool command.
#[derive(Debug)]
pub struct ToolResult {
    pub language: String,
    pub phase: String, // "format", "lint", "typecheck"
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ToolResult {
    pub fn passed(&self) -> bool {
        self.exit_code == 0
    }
}

/// Load toolchain config. Priority: repo-local > global defaults.
pub fn load_toolchain(worktree_path: &Path, global_defaults: &ToolchainConfig) -> ToolchainConfig {
    let repo_config_path = worktree_path.join(".reckoner/toolchain.toml");
    if repo_config_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&repo_config_path) {
            if let Ok(config) = toml::from_str::<ToolchainConfig>(&content) {
                tracing::info!("loaded repo toolchain config from .reckoner/toolchain.toml");
                return config;
            }
        }
    }

    // Fall back to auto-detection + global defaults
    let detected = detect_languages(worktree_path);
    let mut config = ToolchainConfig::new();
    for lang in detected {
        if let Some(defaults) = global_defaults.get(&lang) {
            config.insert(lang, defaults.clone());
        }
    }

    if config.is_empty() {
        tracing::info!("no toolchain configured and no languages detected");
    } else {
        let langs: Vec<_> = config.keys().collect();
        tracing::info!(languages = ?langs, "auto-detected toolchain from global defaults");
    }

    config
}

/// Detect languages present in the worktree by checking for marker files.
fn detect_languages(worktree_path: &Path) -> Vec<String> {
    let mut langs = Vec::new();

    // Python: pyproject.toml, setup.py, requirements.txt, or *.py files
    if worktree_path.join("pyproject.toml").exists()
        || worktree_path.join("setup.py").exists()
        || worktree_path.join("requirements.txt").exists()
    {
        langs.push("python".into());
    }

    // TypeScript/JavaScript: package.json, tsconfig.json
    if worktree_path.join("package.json").exists() || worktree_path.join("tsconfig.json").exists() {
        langs.push("typescript".into());
    }

    // Rust: Cargo.toml
    if worktree_path.join("Cargo.toml").exists() {
        langs.push("rust".into());
    }

    // Go: go.mod
    if worktree_path.join("go.mod").exists() {
        langs.push("go".into());
    }

    langs
}

/// Run the toolchain phases (format → lint → typecheck) and collect results.
pub fn run_toolchain(worktree_path: &Path, config: &ToolchainConfig) -> Vec<ToolResult> {
    let mut results = Vec::new();

    for (lang, tools) in config {
        // Run in order: format first (auto-fixes), then lint, then typecheck
        for (phase, cmd_opt) in [
            ("format", &tools.format),
            ("lint", &tools.lint),
            ("typecheck", &tools.typecheck),
        ] {
            if let Some(cmd) = cmd_opt {
                let result = run_tool_command(worktree_path, lang, phase, cmd);
                results.push(result);
            }
        }
    }

    results
}

fn run_tool_command(worktree_path: &Path, language: &str, phase: &str, cmd: &str) -> ToolResult {
    tracing::info!(language, phase, cmd, "running toolchain command");

    let output = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(worktree_path)
        .output();

    match output {
        Ok(out) => ToolResult {
            language: language.into(),
            phase: phase.into(),
            command: cmd.into(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into(),
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        },
        Err(e) => ToolResult {
            language: language.into(),
            phase: phase.into(),
            command: cmd.into(),
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("failed to run command: {}", e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detect_rust_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&"rust".to_string()));
    }

    #[test]
    fn detect_python_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&"python".to_string()));
    }

    #[test]
    fn detect_typescript_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&"typescript".to_string()));
    }

    #[test]
    fn detect_no_languages() {
        let dir = TempDir::new().unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.is_empty());
    }

    #[test]
    fn load_toolchain_uses_repo_config() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".reckoner")).unwrap();
        std::fs::write(
            dir.path().join(".reckoner/toolchain.toml"),
            r#"
[python]
lint = "ruff check ."
format = "ruff format ."
"#,
        )
        .unwrap();

        let defaults = ToolchainConfig::new();
        let config = load_toolchain(dir.path(), &defaults);
        assert!(config.contains_key("python"));
        assert_eq!(config["python"].lint.as_deref(), Some("ruff check ."));
    }

    #[test]
    fn load_toolchain_falls_back_to_defaults() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let mut defaults = ToolchainConfig::new();
        defaults.insert(
            "rust".into(),
            LanguageTools {
                lint: Some("cargo clippy".into()),
                format: Some("cargo fmt".into()),
                typecheck: None,
            },
        );

        let config = load_toolchain(dir.path(), &defaults);
        assert!(config.contains_key("rust"));
        assert_eq!(config["rust"].lint.as_deref(), Some("cargo clippy"));
    }

    #[test]
    fn run_tool_command_captures_output() {
        let dir = TempDir::new().unwrap();
        let result = run_tool_command(dir.path(), "test", "lint", "echo hello");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn run_tool_command_captures_failure() {
        let dir = TempDir::new().unwrap();
        let result = run_tool_command(dir.path(), "test", "lint", "false");
        assert_ne!(result.exit_code, 0);
    }
}
