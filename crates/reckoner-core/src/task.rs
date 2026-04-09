use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::Config;
use crate::container::{ContainerSpec, DockerRuntime};
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

/// Run a complete task lifecycle: provision -> run PAS -> collect -> teardown.
pub async fn run_task(
    config: &Config,
    db_path: &Path,
    repo_name: &str,
    prompt: &str,
    pipeline: Option<&str>,
) -> anyhow::Result<String> {
    let task_id = gen_task_id();
    tracing::info!(task_id, repo = repo_name, "starting task");

    // Look up the repo
    let db = Db::open(db_path)?;
    let repo = db
        .get_repo_by_name(repo_name)?
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found. Run `reck add` first.", repo_name))?;

    db.insert_task(&task_id, repo.id, prompt)?;
    drop(db); // release connection during long-running operations

    // ── 1. PROVISION ─────────────────────────────────────────────────

    {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "pending", "provisioning", None)?;
    }

    // Fetch latest
    let bare_path = PathBuf::from(&repo.local_path);
    if let Err(e) = repo::fetch(&bare_path) {
        let db = Db::open(db_path)?;
        db.set_task_error(&task_id, "provisioning", &e.to_string())?;
        db.transition_task(&task_id, "provisioning", "failed", Some(&e.to_string()))?;
        return Err(e);
    }

    // Create worktree
    let branch_name = repo::task_branch_name(&config.git.pr_prefix, &task_id, prompt);
    let worktree_path = match repo::worktree_add(
        &bare_path,
        &config.general.worktrees_dir,
        &branch_name,
        &repo.default_branch,
    ) {
        Ok(p) => p,
        Err(e) => {
            let db = Db::open(db_path)?;
            db.set_task_error(&task_id, "provisioning", &e.to_string())?;
            db.transition_task(&task_id, "provisioning", "failed", Some(&e.to_string()))?;
            return Err(e);
        }
    };

    // Create logs directory
    let logs_path = config.general.logs_dir.join(&task_id);
    std::fs::create_dir_all(&logs_path)?;

    {
        let db = Db::open(db_path)?;
        db.set_task_branch(&task_id, &branch_name)?;
    }

    // Create container
    let runtime = DockerRuntime::new()?;
    let container_name = format!("reck-{}", &task_id);
    let spec = ContainerSpec {
        name: container_name.clone(),
        image: config.container.base_image.clone(),
        worktree_path: worktree_path.to_string_lossy().into(),
        logs_path: logs_path.to_string_lossy().into(),
        env: collect_env_vars(),
        memory_bytes: parse_memory(&config.container.default_memory),
        cpu_count: Some(config.container.default_cpus as i64),
        pids_limit: Some(config.container.pids_limit as i64),
        network: Some(config.container.network.clone()),
    };

    let container_id = match runtime.create(&spec).await {
        Ok(id) => id,
        Err(e) => {
            let _ = repo::worktree_remove(&bare_path, &worktree_path);
            let db = Db::open(db_path)?;
            db.set_task_error(&task_id, "provisioning", &e.to_string())?;
            db.transition_task(&task_id, "provisioning", "failed", Some(&e.to_string()))?;
            return Err(e.into());
        }
    };

    if let Err(e) = runtime.start(&container_id).await {
        let _ = runtime.remove(&container_id).await;
        let _ = repo::worktree_remove(&bare_path, &worktree_path);
        let db = Db::open(db_path)?;
        db.set_task_error(&task_id, "provisioning", &e.to_string())?;
        db.transition_task(&task_id, "provisioning", "failed", Some(&e.to_string()))?;
        return Err(e.into());
    }

    {
        let db = Db::open(db_path)?;
        db.set_task_container(&task_id, &container_id.0)?;
    }

    // ── 2. RUN PAS ───────────────────────────────────────────────────

    {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "provisioning", "running", None)?;
    }

    let start_time = Instant::now();

    // Ensure .reckoner dir exists inside container
    let _ = runtime
        .run_command(&container_id, &["mkdir", "-p", "/workspace/.reckoner"])
        .await;

    let result_path = "/workspace/.reckoner/result.json";
    let budget_str = config.pas.default_max_budget_usd.to_string();

    let cmd: Vec<&str> = if let Some(dot_file) = pipeline {
        vec![
            &config.pas.binary,
            "run",
            dot_file,
            "--workdir",
            "/workspace",
            "--output-result",
            result_path,
            "--max-budget-usd",
            &budget_str,
        ]
    } else {
        // No pipeline: use claude directly with the prompt
        vec![
            "claude",
            "-p",
            prompt,
            "--output-format",
            "json",
            "--model",
            &config.pas.default_model,
            "--no-session-persistence",
        ]
    };

    tracing::info!(cmd = ?cmd, "running inside container");
    let run_result = runtime.run_command(&container_id, &cmd).await;
    let duration = start_time.elapsed().as_secs() as i64;

    // Save stdout/stderr to log files
    if let Ok(ref result) = run_result {
        let _ = std::fs::write(logs_path.join("pas-stdout.jsonl"), &result.stdout);
        let _ = std::fs::write(logs_path.join("pas-stderr.log"), &result.stderr);
    }

    match &run_result {
        Ok(result) => {
            let db = Db::open(db_path)?;
            let run_id = db.insert_run(
                &task_id,
                pipeline.unwrap_or("generated"),
                &logs_path.to_string_lossy(),
            )?;
            let status = if result.exit_code == 0 {
                "success"
            } else {
                "partial"
            };
            db.finish_run(run_id, status, 0.0, duration)?;

            if result.exit_code != 0 {
                tracing::warn!(
                    exit_code = result.exit_code,
                    "PAS exited with non-zero code"
                );
            }
        }
        Err(e) => {
            let db = Db::open(db_path)?;
            db.set_task_error(&task_id, "running", &e.to_string())?;
            db.transition_task(&task_id, "running", "failed", Some(&e.to_string()))?;
        }
    }

    // ── 3. COLLECT & TEARDOWN ────────────────────────────────────────

    let container_logs = runtime.collect_logs(&container_id).await.unwrap_or_default();
    let _ = std::fs::write(logs_path.join("container.jsonl"), &container_logs);

    let _ = runtime.stop(&container_id).await;
    let _ = runtime.remove(&container_id).await;
    let _ = repo::worktree_remove(&bare_path, &worktree_path);

    if run_result.is_err() {
        return Err(run_result.unwrap_err().into());
    }

    // Mark done
    {
        let db = Db::open(db_path)?;
        db.transition_task(&task_id, "running", "done", None)?;
    }

    tracing::info!(task_id, duration_secs = duration, "task completed");
    Ok(task_id)
}

/// Collect environment variables to pass into the container.
fn collect_env_vars() -> Vec<String> {
    let mut env = vec![
        "GIT_TERMINAL_PROMPT=0".into(),
        "TERM=xterm-256color".into(),
    ];

    for key in &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GH_TOKEN",
    ] {
        if let Ok(val) = std::env::var(key) {
            env.push(format!("{}={}", key, val));
        }
    }

    env
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
