use reckoner_core::config::Config;
use reckoner_core::db::Db;

pub fn show_all(config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    let tasks = db.list_active_tasks()?;

    if tasks.is_empty() {
        println!("No active tasks.");
        return Ok(());
    }

    println!(
        "{:<14} {:<14} {:<30} {}",
        "TASK", "STATUS", "PROMPT", "CREATED"
    );
    println!("{}", "-".repeat(72));
    for t in &tasks {
        let prompt_short: String = t.prompt.chars().take(28).collect();
        println!(
            "{:<14} {:<14} {:<30} {}",
            t.id, t.status, prompt_short, t.created_at
        );
    }
    Ok(())
}

pub fn show_one(task_id: &str, config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    let task = db
        .get_task(task_id)?
        .ok_or_else(|| anyhow::anyhow!("task '{}' not found", task_id))?;

    println!("Task:       {}", task.id);
    println!("Status:     {}", task.status);
    println!("Prompt:     {}", task.prompt);
    println!("Created:    {}", task.created_at);

    if let Some(ref branch) = task.branch_name {
        println!("Branch:     {}", branch);
    }
    if let Some(ref pr) = task.pr_url {
        println!("PR:         {}", pr);
    }
    if let Some(ref started) = task.started_at {
        println!("Started:    {}", started);
    }
    if let Some(ref completed) = task.completed_at {
        println!("Completed:  {}", completed);
    }
    if task.total_cost_usd > 0.0 {
        println!("Cost:       ${:.2}", task.total_cost_usd);
    }
    if let Some(ref stage) = task.failed_stage {
        println!("Failed at:  {}", stage);
    }
    if let Some(ref msg) = task.error_message {
        println!("Error:      {}", msg);
    }
    println!("Attempts:   {}", task.attempt_count);

    Ok(())
}
