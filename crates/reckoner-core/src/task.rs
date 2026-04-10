use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::config::Config;
use crate::db::Db;
use crate::repo;

/// Valid task status transitions.
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    ("pending", &["provisioning", "failed"]),
    ("provisioning", &["running", "failed"]),
    ("running", &["linting", "done", "failed"]),
    ("linting", &["pr_open", "done", "failed"]),
    ("pr_open", &["done", "failed"]),
    ("done", &[]),
    ("failed", &["pending"]),
];

fn can_transition(from: &str, to: &str) -> bool {
    VALID_TRANSITIONS
        .iter()
        .find(|(s, _)| *s == from)
        .map(|(_, targets)| targets.contains(&to))
        .unwrap_or(false)
}

/// Generate a short task ID.
fn gen_task_id() -> String {
    let id = uuid::Uuid::new_v4();
    format!("reck-{}", &id.to_string()[..8])
}

/// Options for task execution.
pub struct TaskOptions<'a> {
    pub repo_name: &'a str,
    pub prompt: &'a str,
    pub pipeline: Option<&'a str>,
    pub create_pr: bool,
}

/// Run a complete task lifecycle.
///
/// Local mode: runs claude/pas on the HOST (uses Claude subscription auth).
/// The worktree provides file isolation. If `create_pr` is true, commits
/// changes, pushes the branch, and opens a PR via `gh`.
pub async fn run_task(
    config: &Config,
    db_path: &Path,
    opts: &TaskOptions<'_>,
) -> anyhow::Result<String> {
    let task_id = gen_task_id();
    let prompt = opts.prompt;
    let repo_name = opts.repo_name;
    let pipeline = opts.pipeline;
    tracing::info!(task_id, repo = repo_name, "starting task");

    // Look up the repo
    let db = Db::open(db_path)?;
    let r = db
        .get_repo_by_name(repo_name)?
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found. Run `reck add` first.", repo_name))?;

    db.insert_task(&task_id, r.id, prompt)?;
    drop(db);

    // ── 1. PROVISION ─────────────────────────────────────────────────

    {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "pending", "provisioning", None)?;
    }

    let bare_path = PathBuf::from(&r.local_path);
    if let Err(e) = repo::fetch(&bare_path) {
        fail_task(db_path, &task_id, "provisioning", &e)?;
        return Err(e);
    }

    let branch_name = repo::task_branch_name(&config.git.pr_prefix, &task_id, prompt);
    let worktree_path = match repo::worktree_add(
        &bare_path,
        &config.general.worktrees_dir,
        &branch_name,
        &r.default_branch,
    ) {
        Ok(p) => p,
        Err(e) => {
            fail_task(db_path, &task_id, "provisioning", &e)?;
            return Err(e);
        }
    };

    let logs_path = config.general.logs_dir.join(&task_id);
    std::fs::create_dir_all(&logs_path)?;

    {
        let db = Db::open(db_path)?;
        db.set_task_branch(&task_id, &branch_name)?;
    }

    // ── 2. RUN (on host, using Claude subscription) ──────────────────

    {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "provisioning", "running", None)?;
    }

    let start_time = Instant::now();
    let run_result = run_on_host(config, prompt, pipeline, &worktree_path, &logs_path);
    let duration = start_time.elapsed().as_secs() as i64;

    let _exit_code = match run_result {
        Ok(code) => {
            let db = Db::open(db_path)?;
            let run_id = db.insert_run(
                &task_id,
                pipeline.unwrap_or("direct"),
                &logs_path.to_string_lossy(),
            )?;
            let status = if code == 0 { "success" } else { "partial" };
            db.finish_run(run_id, status, 0.0, duration)?;

            if code != 0 {
                tracing::warn!(code, "claude/pas exited with non-zero code");
            }
            code
        }
        Err(e) => {
            fail_task(db_path, &task_id, "running", &e)?;
            let _ = repo::worktree_remove(&bare_path, &worktree_path);
            return Err(e);
        }
    };

    // ── 3. LINT (toolchain + architectural linters + fix loop) ─────────

    if config.linters_enabled() {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "running", "linting", None)?;
        drop(db);

        run_lint_phase(config, &worktree_path, &logs_path)?;

        // After linting (which may have auto-fixed things via toolchain format),
        // transition back. The next phase will pick it up.
    }

    // ── 4. PR (commit + push + open PR) ──────────────────────────────

    if opts.create_pr && config.git.auto_pr {
        if repo::has_changes(&worktree_path)? {
            {
                let db = Db::open(db_path)?;
                // May be coming from "linting" or "running" depending on whether linters ran
                let _ = db.transition_task(&task_id, "linting", "pr_open", None);
                let _ = db.transition_task(&task_id, "running", "pr_open", None);
            }

            let commit_msg = format!("reck: {}", prompt);
            if let Err(e) = repo::commit_all(&worktree_path, &commit_msg, &config.git.commit_author) {
                tracing::warn!(error = %e, "commit failed");
                fail_task(db_path, &task_id, "pr_open", &e)?;
                let _ = repo::worktree_remove(&bare_path, &worktree_path);
                return Err(e);
            }

            if let Err(e) = repo::push(&worktree_path, &branch_name) {
                tracing::warn!(error = %e, "push failed");
                fail_task(db_path, &task_id, "pr_open", &e)?;
                let _ = repo::worktree_remove(&bare_path, &worktree_path);
                return Err(e);
            }

            let diff = repo::diffstat(&worktree_path, &r.default_branch).unwrap_or_default();
            let body = repo::pr_body(&task_id, prompt, &diff);
            let pr_title = format!("{}: {}", config.git.pr_prefix, prompt);

            match repo::create_pr(&worktree_path, &pr_title, &body, &r.default_branch) {
                Ok(pr_url) => {
                    println!("PR: {}", pr_url);
                    let db = Db::open(db_path)?;
                    db.set_task_pr(&task_id, &pr_url)?;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "PR creation failed (changes are pushed)");
                }
            }
        } else {
            tracing::info!("no changes to commit");
        }
    }

    // ── 4. CLEANUP ───────────────────────────────────────────────────

    let _ = repo::worktree_remove(&bare_path, &worktree_path);

    {
        let db = Db::open(db_path)?;
        // Transition from wherever we are to done
        let _ = db.transition_task(&task_id, "pr_open", "done", None);
        let _ = db.transition_task(&task_id, "linting", "done", None);
        let _ = db.transition_task(&task_id, "running", "done", None);
    }

    tracing::info!(task_id, duration_secs = duration, "task completed");
    Ok(task_id)
}

/// Run claude or pas on the HOST against the worktree.
/// This uses the local Claude subscription — no API key needed.
fn run_on_host(
    config: &Config,
    prompt: &str,
    pipeline: Option<&str>,
    worktree_path: &Path,
    logs_path: &Path,
) -> anyhow::Result<i32> {
    let (program, args) = if let Some(dot_file) = pipeline {
        let budget = config.pas.default_max_budget_usd.to_string();
        let max_steps = config.pas.default_max_steps.to_string();
        (
            config.pas.binary.clone(),
            vec![
                "run".into(),
                dot_file.into(),
                "--workdir".into(),
                worktree_path.to_string_lossy().into(),
                "--max-budget-usd".into(),
                budget,
                "--max-steps".into(),
                max_steps,
            ],
        )
    } else {
        (
            "claude".into(),
            vec![
                "-p".into(),
                prompt.into(),
                "--output-format".into(),
                "json".into(),
                "--model".into(),
                config.pas.default_model.clone(),
                "--no-session-persistence".into(),
                "--dangerously-skip-permissions".into(),
            ],
        )
    };

    tracing::info!(program, args = ?args, workdir = %worktree_path.display(), "running on host");

    let output = Command::new(&program)
        .args(&args)
        .current_dir(worktree_path)
        .output()?;

    // Save stdout/stderr to log files
    let _ = std::fs::write(logs_path.join("stdout.jsonl"), &output.stdout);
    let _ = std::fs::write(logs_path.join("stderr.log"), &output.stderr);

    let exit_code = output.status.code().unwrap_or(-1);

    if !output.stdout.is_empty() {
        let preview: String = String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(200)
            .collect();
        tracing::debug!(preview, "stdout preview");
    }

    if exit_code != 0 {
        let stderr_preview: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(500)
            .collect();
        tracing::warn!(exit_code, stderr = %stderr_preview, "non-zero exit");
    }

    Ok(exit_code)
}

/// Run the lint phase: toolchain (format/lint/typecheck) + architectural linters.
/// Saves results to logs. Does NOT run the fix loop yet (that requires Claude).
fn run_lint_phase(
    config: &Config,
    worktree_path: &Path,
    logs_path: &Path,
) -> anyhow::Result<()> {
    // 1. Toolchain: format → lint → typecheck
    let tc_config = crate::toolchain::load_toolchain(worktree_path, config.toolchain_defaults());
    if !tc_config.is_empty() {
        let results = crate::toolchain::run_toolchain(worktree_path, &tc_config);
        let mut toolchain_log = String::new();
        for r in &results {
            let status = if r.passed() { "PASS" } else { "FAIL" };
            let line = format!(
                "{{\"phase\":\"{}\",\"language\":\"{}\",\"command\":\"{}\",\"status\":\"{}\",\"exit_code\":{}}}\n",
                r.phase, r.language, r.command.replace('"', "\\\""), status, r.exit_code
            );
            toolchain_log.push_str(&line);

            if r.passed() {
                tracing::info!(language = r.language, phase = r.phase, "toolchain: passed");
            } else {
                tracing::warn!(
                    language = r.language,
                    phase = r.phase,
                    exit_code = r.exit_code,
                    "toolchain: failed"
                );
            }
        }
        let _ = std::fs::write(logs_path.join("toolchain.jsonl"), &toolchain_log);
    }

    // 2. Architectural linters
    let report = crate::lint::run_linters(worktree_path, config)?;

    if !report.findings.is_empty() {
        // Write findings as JSONL
        let mut lint_log = String::new();
        for f in &report.findings {
            if let Ok(json) = serde_json::to_string(f) {
                lint_log.push_str(&json);
                lint_log.push('\n');
            }
        }
        let _ = std::fs::write(logs_path.join("linter.jsonl"), &lint_log);

        tracing::info!(summary = %report.summary(), "architectural lint results");

        if !report.passed() {
            let prompt = report.remediation_prompt();
            tracing::warn!(
                failures = report.failures().len(),
                "lint failures found — remediation prompt saved to logs"
            );
            let _ = std::fs::write(logs_path.join("lint-remediation.md"), &prompt);
        }
    } else {
        tracing::info!("no architectural lint findings");
    }

    Ok(())
}

/// Helper to record a failure and transition to failed state.
fn fail_task(db_path: &Path, task_id: &str, stage: &str, err: &anyhow::Error) -> anyhow::Result<()> {
    let db = Db::open(db_path)?;
    db.set_task_error(task_id, stage, &err.to_string())?;
    // Try the most likely transition; if it fails (wrong from-state), that's ok
    let from = match stage {
        "provisioning" => "provisioning",
        "running" => "running",
        _ => stage,
    };
    let _ = db.transition_task(task_id, from, "failed", Some(&err.to_string()));
    Ok(())
}

/// Parse a memory string like "4g" into bytes.
fn parse_memory(s: &str) -> Option<i64> {
    let s = s.to_lowercase();
    if let Some(num) = s.strip_suffix('g') {
        num.parse::<i64>().ok().map(|n| n * 1024 * 1024 * 1024)
    } else if let Some(num) = s.strip_suffix('m') {
        num.parse::<i64>().ok().map(|n| n * 1024 * 1024)
    } else {
        s.parse::<i64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions_allow_forward_progress() {
        assert!(can_transition("pending", "provisioning"));
        assert!(can_transition("provisioning", "running"));
        assert!(can_transition("running", "done"));
        assert!(can_transition("running", "failed"));
    }

    #[test]
    fn invalid_transitions_rejected() {
        assert!(!can_transition("pending", "done"));
        assert!(!can_transition("done", "running"));
        assert!(!can_transition("running", "pending"));
    }

    #[test]
    fn failed_can_retry() {
        assert!(can_transition("failed", "pending"));
    }

    #[test]
    fn parse_memory_values() {
        assert_eq!(parse_memory("4g"), Some(4 * 1024 * 1024 * 1024));
        assert_eq!(parse_memory("512m"), Some(512 * 1024 * 1024));
        assert_eq!(parse_memory("1073741824"), Some(1073741824));
    }
}
