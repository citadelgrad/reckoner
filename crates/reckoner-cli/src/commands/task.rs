use reckoner_core::config::Config;
use reckoner_core::task::{self, TaskOptions};

pub async fn run(
    repo_name: &str,
    prompt: &str,
    pipeline: Option<&str>,
    create_pr: bool,
    keep_worktree: bool,
    config: &Config,
) -> anyhow::Result<()> {
    let opts = TaskOptions {
        repo_name,
        prompt,
        pipeline,
        create_pr,
        keep_worktree,
    };
    let task_id = task::run_task(config, &config.general.db_path, &opts).await?;
    println!("Task {} completed", task_id);
    Ok(())
}
