use reckoner_core::config::Config;
use reckoner_core::db::Db;
use reckoner_core::schedule;

pub fn add(
    name: &str,
    repo_name: &str,
    pipeline: &str,
    cron_expr: &str,
    config: &Config,
) -> anyhow::Result<()> {
    // Verify repo exists
    let db = Db::open(&config.general.db_path)?;
    let _repo = db
        .get_repo_by_name(repo_name)?
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found", repo_name))?;

    // Find the reck binary path
    let reck_binary = std::env::current_exe()?
        .to_string_lossy()
        .into_owned();

    // Build and write plist
    let agent = schedule::build_plist(
        name,
        &reck_binary,
        repo_name,
        pipeline,
        cron_expr,
        &config.general.logs_dir,
    )?;

    // Ensure schedule log directory exists
    std::fs::create_dir_all(config.general.logs_dir.join("schedules"))?;

    let plist_path = schedule::write_plist(&agent)?;

    // Load into launchd
    match schedule::launchctl_load(&plist_path) {
        Ok(()) => println!("Schedule '{}' loaded", name),
        Err(e) => println!("Plist written but launchctl load failed: {}\nManually load with: launchctl load {}", e, plist_path.display()),
    }

    println!("  Label:    {}", agent.label);
    println!("  Cron:     {}", cron_expr);
    println!("  Pipeline: {}", pipeline);
    println!("  Repo:     {}", repo_name);
    println!("  Plist:    {}", plist_path.display());

    Ok(())
}

pub fn list() -> anyhow::Result<()> {
    let schedules = schedule::list_schedules()?;

    if schedules.is_empty() {
        println!("No schedules found. Run `reck schedule add` to create one.");
        return Ok(());
    }

    println!("{:<25} {}", "NAME", "PLIST");
    println!("{}", "-".repeat(60));
    for (name, path) in &schedules {
        println!("{:<25} {}", name, path.display());
    }

    Ok(())
}

pub fn remove(name: &str) -> anyhow::Result<()> {
    schedule::remove_schedule(name)?;
    println!("Removed schedule '{}'", name);
    Ok(())
}

pub fn run_now(
    name: &str,
    repo_name: &str,
    pipeline: &str,
    _config: &Config,
) -> anyhow::Result<()> {
    println!("Running schedule '{}' manually...", name);
    // Just delegate to reck task --no-pr
    let status = std::process::Command::new(std::env::current_exe()?)
        .args(["task", repo_name, &format!("scheduled: {}", name), "--pipeline", pipeline, "--no-pr"])
        .status()?;

    if status.success() {
        println!("Schedule '{}' completed successfully", name);
    } else {
        println!("Schedule '{}' failed (exit code: {:?})", name, status.code());
    }

    Ok(())
}
