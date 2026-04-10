use reckoner_core::config::Config;
use std::process::Command;

pub fn run(config: &Config) -> anyhow::Result<()> {
    let mut ok = true;

    // Check git
    ok &= check("git", &["--version"]);

    // Check gh
    ok &= check("gh", &["--version"]);

    // Check pas
    ok &= check(&config.pas.binary, &["--version"]);

    // Check Docker / OrbStack
    ok &= check("docker", &["version", "--format", "{{.Server.Version}}"]);

    // Check API keys
    ok &= check_env("ANTHROPIC_API_KEY");

    // Check database
    let db_exists = config.general.db_path.exists();
    print_check(
        "SQLite database",
        db_exists,
        &config.general.db_path.to_string_lossy(),
    );

    // Check repos dir
    let repos_exists = config.general.repos_dir.exists();
    print_check(
        "Repos directory",
        repos_exists,
        &config.general.repos_dir.to_string_lossy(),
    );

    if ok {
        println!("\nAll checks passed.");
    } else {
        println!("\nSome checks failed. Fix the issues above.");
    }

    Ok(())
}

fn check(binary: &str, args: &[&str]) -> bool {
    match Command::new(binary).args(args).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.trim().lines().next().unwrap_or("ok");
            println!("  [ok] {}: {}", binary, version);
            true
        }
        Ok(_) => {
            println!("  [FAIL] {}: found but returned error", binary);
            false
        }
        Err(_) => {
            println!("  [FAIL] {}: not found on PATH", binary);
            false
        }
    }
}

fn check_env(key: &str) -> bool {
    if std::env::var(key).is_ok() {
        println!("  [ok] {}: set", key);
        true
    } else {
        println!("  [WARN] {}: not set", key);
        false
    }
}

fn print_check(label: &str, ok: bool, detail: &str) {
    if ok {
        println!("  [ok] {}: {}", label, detail);
    } else {
        println!("  [WARN] {}: not found at {}", label, detail);
    }
}
