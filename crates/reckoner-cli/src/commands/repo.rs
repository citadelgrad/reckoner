use reckoner_core::config::Config;
use reckoner_core::db::Db;
use reckoner_core::repo;

pub fn add(url: &str, config: &Config) -> anyhow::Result<()> {
    let bare_path = repo::clone_bare(url, config)?;
    let default_branch = repo::detect_default_branch(&bare_path)?;
    let name = repo::name_from_url(url);

    let db = Db::open(&config.general.db_path)?;
    db.insert_repo(url, &name, &bare_path.to_string_lossy(), &default_branch)?;

    println!("Added {} (branch: {})", name, default_branch);
    println!("  clone: {}", bare_path.display());
    Ok(())
}

pub fn list(config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    let repos = db.list_repos()?;

    if repos.is_empty() {
        println!("No repos registered. Run `reck add <git-url>` to add one.");
        return Ok(());
    }

    println!("{:<20} {:<12} {:<30}", "NAME", "BRANCH", "URL");
    println!("{}", "-".repeat(62));
    for r in &repos {
        let _synced = r.last_synced.as_deref().unwrap_or("never");
        println!("{:<20} {:<12} {}", r.name, r.default_branch, r.url);
    }
    Ok(())
}

pub fn remove(name: &str, config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    if db.remove_repo(name)? {
        println!("Removed {}", name);
    } else {
        anyhow::bail!("repo '{}' not found", name);
    }
    Ok(())
}

pub fn sync(name: &str, config: &Config) -> anyhow::Result<()> {
    let db = Db::open(&config.general.db_path)?;
    let r = db
        .get_repo_by_name(name)?
        .ok_or_else(|| anyhow::anyhow!("repo '{}' not found", name))?;

    let bare_path = std::path::PathBuf::from(&r.local_path);
    repo::fetch(&bare_path)?;
    db.update_repo_synced(r.id)?;

    println!("Synced {}", name);
    Ok(())
}
