use reckoner_core::config::Config;
use reckoner_core::task;

pub async fn run(
    repo_name: &str,
    prompt: &str,
    pipeline: Option<&str>,
    config: &Config,
) -> anyhow::Result<()> {
    let task_id = task::run_task(config, &config.general.db_path, repo_name, prompt, pipeline).await?;
    println!("Task {} completed", task_id);
    Ok(())
}
