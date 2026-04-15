use reckoner_core::config::Config;
use reckoner_core::db::Db;
use reckoner_core::lint;
use reckoner_core::toolchain;

pub fn run(repo_name: &str, config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    let repo = db
        .get_repo_by_name(repo_name)?
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found", repo_name))?;

    let bare_path = std::path::PathBuf::from(&repo.local_path);

    // Use a temporary worktree for linting (read-only check against current HEAD)
    let branch = format!(
        "reckoner/lint-{}",
        uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
    );
    let worktree_path = reckoner_core::repo::worktree_add(
        &bare_path,
        &config.general.worktrees_dir,
        &branch,
        &repo.default_branch,
    )?;

    // Run linting in a closure so cleanup always happens
    let result = (|| -> anyhow::Result<()> {
        // Toolchain
        let tc_config = toolchain::load_toolchain(&worktree_path, config.toolchain_defaults());
        if !tc_config.is_empty() {
            println!("Running toolchain...");
            let results = toolchain::run_toolchain(&worktree_path, &tc_config);
            for r in &results {
                let icon = if r.passed() { "  [ok]" } else { "  [FAIL]" };
                println!("{} {}/{}: {}", icon, r.language, r.phase, r.command);
                if !r.passed() && !r.stderr.is_empty() {
                    // Show first few lines of stderr
                    for line in r.stderr.lines().take(5) {
                        println!("       {}", line);
                    }
                }
            }
        }

        // Architectural linters
        println!("\nRunning architectural linters...");
        let report = lint::run_linters(&worktree_path, config)?;

        if report.findings.is_empty() {
            println!("  No findings.");
        } else {
            for f in &report.findings {
                let icon = match f.status.as_str() {
                    "fail" => "[FAIL]",
                    "warn" => "[WARN]",
                    _ => "[ ok ]",
                };
                print!("  {} [{}] {}", icon, f.rule, f.file);
                if let Some(line) = f.line {
                    print!(":{}", line);
                }
                println!();
                println!("       {}", f.message);
                if f.status == "fail" {
                    println!("       Fix: {}", f.remediation);
                }
            }
            println!("\n{}", report.summary());
        }

        Ok(())
    })();

    // Always cleanup, even if linting errored
    let _ = reckoner_core::repo::worktree_remove(&bare_path, &worktree_path);
    let _ = reckoner_core::repo::branch_delete(&bare_path, &branch);

    result
}
