use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use crate::config::Config;
use crate::lint::{self, LintFinding, LintReport};

/// Result of a single fix iteration.
#[derive(Debug)]
pub struct FixIteration {
    pub iteration: u32,
    pub failures_before: usize,
    pub failures_after: usize,
    pub stuck_violations: Vec<String>,
    pub claude_exit_code: i32,
}

/// Result of the entire lint-fix loop.
#[derive(Debug)]
pub struct FixLoopResult {
    pub iterations_run: u32,
    pub max_iterations: u32,
    pub final_failures: usize,
    pub all_passed: bool,
    pub stuck_violations: Vec<String>,
    pub history: Vec<FixIteration>,
}

/// Fingerprint a lint finding for stuck-violation tracking.
fn finding_key(f: &LintFinding) -> String {
    format!("{}:{}:{}", f.rule, f.file, f.line.unwrap_or(0))
}

/// Run the lint-fix loop: lint → fix → re-lint, up to max_iterations.
pub fn run_fix_loop(
    config: &Config,
    worktree_path: &Path,
    logs_path: &Path,
    model: &str,
) -> anyhow::Result<FixLoopResult> {
    let max_iter = config.linters.max_fix_iterations;
    let mut history = Vec::new();
    let mut previous_keys: HashSet<String> = HashSet::new();
    let mut all_stuck: Vec<String> = Vec::new();

    for iteration in 1..=max_iter {
        tracing::info!(iteration, max_iter, "lint-fix loop iteration");

        // Run linters
        let report = lint::run_linters(worktree_path, config)?;
        let failures = report.failures();
        let failure_count = failures.len();

        if failure_count == 0 {
            tracing::info!(iteration, "all lints pass — exiting fix loop");
            history.push(FixIteration {
                iteration,
                failures_before: 0,
                failures_after: 0,
                stuck_violations: vec![],
                claude_exit_code: 0,
            });
            return Ok(FixLoopResult {
                iterations_run: iteration,
                max_iterations: max_iter,
                final_failures: 0,
                all_passed: true,
                stuck_violations: all_stuck,
                history,
            });
        }

        // Detect stuck violations (same finding persists across iterations)
        let current_keys: HashSet<String> = failures.iter().map(|f| finding_key(f)).collect();
        let stuck: Vec<String> = current_keys
            .intersection(&previous_keys)
            .cloned()
            .collect();

        if !stuck.is_empty() {
            tracing::warn!(
                count = stuck.len(),
                "stuck violations detected — same findings persist"
            );
            all_stuck.extend(stuck.clone());
        }

        // Build remediation prompt
        let prompt = report.remediation_prompt();
        let _ = std::fs::write(
            logs_path.join(format!("fix-iteration-{}.md", iteration)),
            &prompt,
        );

        // Run Claude to fix
        let exit_code = run_claude_fix(worktree_path, &prompt, model, logs_path, iteration)?;

        // Re-lint to check results
        let after_report = lint::run_linters(worktree_path, config)?;
        let after_failures = after_report.failures().len();

        history.push(FixIteration {
            iteration,
            failures_before: failure_count,
            failures_after: after_failures,
            stuck_violations: stuck,
            claude_exit_code: exit_code,
        });

        previous_keys = current_keys;

        if after_failures == 0 {
            tracing::info!(iteration, "all lints pass after fix");
            return Ok(FixLoopResult {
                iterations_run: iteration,
                max_iterations: max_iter,
                final_failures: 0,
                all_passed: true,
                stuck_violations: all_stuck,
                history,
            });
        }
    }

    // Exhausted iterations
    let final_report = lint::run_linters(worktree_path, config)?;
    let final_failures = final_report.failures().len();

    tracing::warn!(
        iterations = max_iter,
        remaining_failures = final_failures,
        "lint-fix loop exhausted"
    );

    Ok(FixLoopResult {
        iterations_run: max_iter,
        max_iterations: max_iter,
        final_failures,
        all_passed: false,
        stuck_violations: all_stuck,
        history,
    })
}

/// Run Claude to fix lint violations. Returns exit code.
fn run_claude_fix(
    worktree_path: &Path,
    remediation_prompt: &str,
    model: &str,
    logs_path: &Path,
    iteration: u32,
) -> anyhow::Result<i32> {
    let full_prompt = format!(
        "You are fixing lint violations in code you previously generated. \
         Make the minimum change to resolve each violation. \
         Do not refactor unrelated code.\n\n{}",
        remediation_prompt
    );

    tracing::info!(iteration, model, "running Claude to fix lint violations");

    let output = Command::new("claude")
        .args([
            "-p",
            &full_prompt,
            "--output-format",
            "json",
            "--model",
            model,
            "--no-session-persistence",
            "--dangerously-skip-permissions",
        ])
        .current_dir(worktree_path)
        .output()?;

    let _ = std::fs::write(
        logs_path.join(format!("fix-claude-{}.jsonl", iteration)),
        &output.stdout,
    );

    Ok(output.status.code().unwrap_or(-1))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── finding_key ──────────────────────────────────────────────────

    #[test]
    fn finding_key_includes_rule_file_line() {
        let f = LintFinding {
            rule: "file-size".into(),
            status: "fail".into(),
            level: "warning".into(),
            file: "src/main.rs".into(),
            line: Some(42),
            message: "too big".into(),
            remediation: "split".into(),
            context: serde_json::Value::Null,
        };
        assert_eq!(finding_key(&f), "file-size:src/main.rs:42");
    }

    #[test]
    fn finding_key_handles_no_line() {
        let f = LintFinding {
            rule: "file-size".into(),
            status: "fail".into(),
            level: "warning".into(),
            file: "big.rs".into(),
            line: None,
            message: "".into(),
            remediation: "".into(),
            context: serde_json::Value::Null,
        };
        assert_eq!(finding_key(&f), "file-size:big.rs:0");
    }

    // ── FixLoopResult ────────────────────────────────────────────────

    #[test]
    fn fix_loop_result_all_passed() {
        let result = FixLoopResult {
            iterations_run: 1,
            max_iterations: 3,
            final_failures: 0,
            all_passed: true,
            stuck_violations: vec![],
            history: vec![],
        };
        assert!(result.all_passed);
        assert_eq!(result.final_failures, 0);
    }

    #[test]
    fn fix_loop_result_exhausted() {
        let result = FixLoopResult {
            iterations_run: 3,
            max_iterations: 3,
            final_failures: 2,
            all_passed: false,
            stuck_violations: vec!["file-size:big.rs:0".into()],
            history: vec![
                FixIteration {
                    iteration: 1,
                    failures_before: 3,
                    failures_after: 2,
                    stuck_violations: vec![],
                    claude_exit_code: 0,
                },
                FixIteration {
                    iteration: 2,
                    failures_before: 2,
                    failures_after: 2,
                    stuck_violations: vec!["file-size:big.rs:0".into()],
                    claude_exit_code: 0,
                },
                FixIteration {
                    iteration: 3,
                    failures_before: 2,
                    failures_after: 2,
                    stuck_violations: vec!["file-size:big.rs:0".into()],
                    claude_exit_code: 0,
                },
            ],
        };
        assert!(!result.all_passed);
        assert_eq!(result.iterations_run, 3);
        assert_eq!(result.final_failures, 2);
        assert!(!result.stuck_violations.is_empty());
    }

    #[test]
    fn fix_iteration_tracks_progress() {
        let iter = FixIteration {
            iteration: 1,
            failures_before: 5,
            failures_after: 2,
            stuck_violations: vec![],
            claude_exit_code: 0,
        };
        assert!(iter.failures_after < iter.failures_before);
    }

    // ── stuck violation detection ────────────────────────────────────

    #[test]
    fn stuck_detection_finds_repeated_keys() {
        let prev: HashSet<String> = ["a:b:1".into(), "c:d:2".into()].into();
        let curr: HashSet<String> = ["a:b:1".into(), "e:f:3".into()].into();
        let stuck: Vec<String> = curr.intersection(&prev).cloned().collect();
        assert_eq!(stuck, vec!["a:b:1"]);
    }

    #[test]
    fn stuck_detection_empty_when_all_new() {
        let prev: HashSet<String> = ["a:b:1".into()].into();
        let curr: HashSet<String> = ["c:d:2".into()].into();
        let stuck: Vec<String> = curr.intersection(&prev).cloned().collect();
        assert!(stuck.is_empty());
    }
}
