use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single lint finding in JSON-Lines format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintFinding {
    pub rule: String,
    pub status: String, // "pass", "warn", "fail"
    pub level: String,  // "error", "warning", "info"
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
    pub remediation: String,
    #[serde(default)]
    pub context: serde_json::Value,
}

/// Aggregated results from running all linters.
#[derive(Debug, Default)]
pub struct LintReport {
    pub findings: Vec<LintFinding>,
}

impl LintReport {
    pub fn failures(&self) -> Vec<&LintFinding> {
        self.findings.iter().filter(|f| f.status == "fail").collect()
    }

    pub fn warnings(&self) -> Vec<&LintFinding> {
        self.findings.iter().filter(|f| f.status == "warn").collect()
    }

    pub fn passed(&self) -> bool {
        self.failures().is_empty()
    }

    pub fn summary(&self) -> String {
        let fails = self.failures().len();
        let warns = self.warnings().len();
        let passes = self.findings.len() - fails - warns;
        format!("{} passed, {} warnings, {} failures", passes, warns, fails)
    }

    /// Format failures as a prompt for the lint-fix loop.
    pub fn remediation_prompt(&self) -> String {
        let failures = self.failures();
        if failures.is_empty() {
            return String::new();
        }

        let mut prompt = String::from(
            "The following lint violations were found. Fix each one. \
             The remediation field tells you how.\n\n",
        );

        for (i, f) in failures.iter().enumerate() {
            prompt.push_str(&format!(
                "Violation {}: [{}] {}\n  File: {}",
                i + 1,
                f.rule,
                f.message,
                f.file,
            ));
            if let Some(line) = f.line {
                prompt.push_str(&format!(":{}", line));
            }
            prompt.push_str(&format!("\n  Remediation: {}\n\n", f.remediation));
        }

        prompt
    }
}

/// Run all linters against a worktree. Discovers built-in linters and
/// external executables in .reckoner/linters/ and ~/.reckoner/linters/.
pub fn run_linters(
    worktree_path: &Path,
    config: &crate::config::Config,
) -> anyhow::Result<LintReport> {
    let mut report = LintReport::default();

    // Built-in: file-size
    if config.linters_enabled() {
        let max_lines = config.linter_max_lines();
        let findings = lint_file_size(worktree_path, max_lines)?;
        report.findings.extend(findings);
    }

    // External linters: scan directories for executables
    let search_dirs = external_linter_dirs(worktree_path, config);
    for dir in search_dirs {
        if dir.exists() {
            let findings = run_external_linters(&dir, worktree_path)?;
            report.findings.extend(findings);
        }
    }

    Ok(report)
}

/// Built-in file-size linter: flag files exceeding max_lines.
fn lint_file_size(worktree_path: &Path, max_lines: u32) -> anyhow::Result<Vec<LintFinding>> {
    let mut findings = Vec::new();

    let output = std::process::Command::new("find")
        .args([
            worktree_path.to_str().unwrap_or("."),
            "-type", "f",
            "-name", "*.rs",
            "-o", "-name", "*.py",
            "-o", "-name", "*.ts",
            "-o", "-name", "*.tsx",
            "-o", "-name", "*.js",
            "-o", "-name", "*.go",
        ])
        .output()?;

    let files = String::from_utf8_lossy(&output.stdout);
    for file_path in files.lines().filter(|l| !l.is_empty()) {
        let path = Path::new(file_path);

        // Skip hidden dirs and common non-source dirs
        let path_str = file_path;
        if path_str.contains("/.git/")
            || path_str.contains("/node_modules/")
            || path_str.contains("/target/")
            || path_str.contains("/__pycache__/")
            || path_str.contains("/.venv/")
        {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            let line_count = content.lines().count() as u32;
            if line_count > max_lines {
                let relative = path
                    .strip_prefix(worktree_path)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned();

                findings.push(LintFinding {
                    rule: "file-size".into(),
                    status: "fail".into(),
                    level: "warning".into(),
                    file: relative,
                    line: None,
                    message: format!(
                        "File has {} lines, exceeding the {} line limit.",
                        line_count, max_lines
                    ),
                    remediation: format!(
                        "Split this file into smaller modules. Look for natural extraction \
                         points — large functions, distinct responsibilities, or logical groupings. \
                         Target {} lines or fewer per file.",
                        max_lines
                    ),
                    context: serde_json::json!({
                        "line_count": line_count,
                        "max_lines": max_lines
                    }),
                });
            }
        }
    }

    Ok(findings)
}

/// Directories to search for external linter executables.
fn external_linter_dirs(worktree_path: &Path, _config: &crate::config::Config) -> Vec<PathBuf> {
    let mut dirs = vec![
        worktree_path.join(".reckoner/linters"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".reckoner/linters"));
    }
    dirs
}

/// Run all executables in a linter directory, collect JSON-Lines output.
fn run_external_linters(
    linter_dir: &Path,
    worktree_path: &Path,
) -> anyhow::Result<Vec<LintFinding>> {
    let mut findings = Vec::new();

    let entries = std::fs::read_dir(linter_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check if executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&path)?.permissions();
            if perms.mode() & 0o111 == 0 {
                continue; // not executable
            }
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        tracing::info!(linter = name, "running external linter");

        let output = std::process::Command::new(&path)
            .arg(worktree_path)
            .current_dir(worktree_path)
            .env("RECKONER_REPO_ROOT", worktree_path)
            .output();

        match output {
            Ok(out) => {
                // Parse each line of stdout as a JSON LintFinding
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    if let Ok(finding) = serde_json::from_str::<LintFinding>(line) {
                        findings.push(finding);
                    }
                }

                if out.status.code() == Some(2) {
                    tracing::warn!(linter = name, "linter itself errored (exit code 2)");
                }
            }
            Err(e) => {
                tracing::warn!(linter = name, error = %e, "failed to run external linter");
            }
        }
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lint_report_summary() {
        let report = LintReport {
            findings: vec![
                LintFinding {
                    rule: "file-size".into(),
                    status: "pass".into(),
                    level: "info".into(),
                    file: "small.rs".into(),
                    line: None,
                    message: "ok".into(),
                    remediation: "".into(),
                    context: serde_json::Value::Null,
                },
                LintFinding {
                    rule: "file-size".into(),
                    status: "fail".into(),
                    level: "warning".into(),
                    file: "big.rs".into(),
                    line: None,
                    message: "too big".into(),
                    remediation: "split it".into(),
                    context: serde_json::Value::Null,
                },
            ],
        };

        assert!(!report.passed());
        assert_eq!(report.failures().len(), 1);
        assert_eq!(report.summary(), "1 passed, 0 warnings, 1 failures");
    }

    #[test]
    fn lint_report_remediation_prompt() {
        let report = LintReport {
            findings: vec![LintFinding {
                rule: "file-size".into(),
                status: "fail".into(),
                level: "warning".into(),
                file: "src/big.rs".into(),
                line: Some(1),
                message: "File has 600 lines".into(),
                remediation: "Split into modules".into(),
                context: serde_json::Value::Null,
            }],
        };

        let prompt = report.remediation_prompt();
        assert!(prompt.contains("file-size"));
        assert!(prompt.contains("src/big.rs:1"));
        assert!(prompt.contains("Split into modules"));
    }

    #[test]
    fn empty_report_passes() {
        let report = LintReport::default();
        assert!(report.passed());
        assert_eq!(report.summary(), "0 passed, 0 warnings, 0 failures");
    }

    #[test]
    fn file_size_linter_catches_large_files() {
        let dir = TempDir::new().unwrap();

        // Create a file with 20 lines
        let content = "line\n".repeat(20);
        std::fs::write(dir.path().join("small.rs"), &content).unwrap();

        // Create a file with 100 lines
        let big_content = "line\n".repeat(100);
        std::fs::write(dir.path().join("big.rs"), &big_content).unwrap();

        let findings = lint_file_size(dir.path(), 50).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].file.contains("big.rs"));
        assert_eq!(findings[0].rule, "file-size");
        assert!(findings[0].remediation.contains("50"));
    }

    #[test]
    fn file_size_linter_skips_hidden_dirs() {
        let dir = TempDir::new().unwrap();

        // File in .git should be skipped
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let big = "line\n".repeat(1000);
        std::fs::write(dir.path().join(".git/huge.rs"), &big).unwrap();

        let findings = lint_file_size(dir.path(), 50).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn lint_finding_serializes_to_jsonl() {
        let finding = LintFinding {
            rule: "file-size".into(),
            status: "fail".into(),
            level: "warning".into(),
            file: "src/main.rs".into(),
            line: Some(1),
            message: "too big".into(),
            remediation: "split it".into(),
            context: serde_json::json!({"line_count": 600}),
        };

        let json = serde_json::to_string(&finding).unwrap();
        assert!(json.contains("file-size"));
        assert!(json.contains("600"));

        // Round-trip
        let parsed: LintFinding = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rule, "file-size");
    }
}
