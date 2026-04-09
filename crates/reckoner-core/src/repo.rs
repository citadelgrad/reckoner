use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_in_cwd(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Extract a repo name from a git URL.
/// `git@github.com:user/my-repo.git` -> `my-repo`
/// `https://github.com/user/my-repo.git` -> `my-repo`
pub fn name_from_url(url: &str) -> String {
    let s = url
        .rsplit('/')
        .next()
        .or_else(|| url.rsplit(':').next())
        .unwrap_or(url);
    s.trim_end_matches(".git").to_string()
}

/// Clone a repo as a bare treeless clone for fast initial fetch.
pub fn clone_bare(url: &str, config: &Config) -> anyhow::Result<PathBuf> {
    let name = name_from_url(url);
    let dest = config.general.repos_dir.join(format!("{}.git", name));

    if dest.exists() {
        anyhow::bail!("repo already exists at {}", dest.display());
    }

    std::fs::create_dir_all(&config.general.repos_dir)?;

    tracing::info!(url, dest = %dest.display(), "cloning bare repo");
    git_in_cwd(&[
        "clone",
        "--bare",
        "--filter=blob:none",
        url,
        &dest.to_string_lossy(),
    ])?;

    Ok(dest)
}

/// Detect the default branch of a bare repo.
pub fn detect_default_branch(bare_path: &Path) -> anyhow::Result<String> {
    let output = git(bare_path, &["symbolic-ref", "HEAD"])?;
    // refs/heads/main -> main
    Ok(output
        .strip_prefix("refs/heads/")
        .unwrap_or(&output)
        .to_string())
}

/// Fetch latest changes from origin.
pub fn fetch(bare_path: &Path) -> anyhow::Result<()> {
    tracing::info!(repo = %bare_path.display(), "fetching");
    git(bare_path, &["fetch", "--prune", "origin"])?;
    Ok(())
}

/// Create a worktree for a task, branching from the default branch.
pub fn worktree_add(
    bare_path: &Path,
    worktree_dir: &Path,
    branch_name: &str,
    base_branch: &str,
) -> anyhow::Result<PathBuf> {
    let worktree_path = worktree_dir.join(branch_name.replace('/', "-"));
    std::fs::create_dir_all(worktree_dir)?;

    tracing::info!(
        branch = branch_name,
        path = %worktree_path.display(),
        "creating worktree"
    );
    git(
        bare_path,
        &[
            "worktree",
            "add",
            &worktree_path.to_string_lossy(),
            "-b",
            branch_name,
            &format!("origin/{}", base_branch),
        ],
    )?;

    Ok(worktree_path)
}

/// Remove a worktree after task completion.
pub fn worktree_remove(bare_path: &Path, worktree_path: &Path) -> anyhow::Result<()> {
    tracing::info!(path = %worktree_path.display(), "removing worktree");
    git(
        bare_path,
        &["worktree", "remove", "--force", &worktree_path.to_string_lossy()],
    )?;
    Ok(())
}

/// Generate a branch name for a task.
pub fn task_branch_name(prefix: &str, task_id: &str, prompt: &str) -> String {
    let slug: String = prompt
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    format!("{}/feat/{}-{}", prefix, task_id, slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_ssh_url() {
        assert_eq!(
            name_from_url("git@github.com:user/my-repo.git"),
            "my-repo"
        );
    }

    #[test]
    fn name_from_https_url() {
        assert_eq!(
            name_from_url("https://github.com/user/my-repo.git"),
            "my-repo"
        );
    }

    #[test]
    fn name_from_url_no_git_suffix() {
        assert_eq!(
            name_from_url("https://github.com/user/my-repo"),
            "my-repo"
        );
    }

    #[test]
    fn branch_name_generation() {
        let branch = task_branch_name("reckoner", "reck-42", "add user authentication");
        assert_eq!(branch, "reckoner/feat/reck-42-add-user-authentication");
    }

    #[test]
    fn branch_name_truncates_long_prompts() {
        let branch = task_branch_name(
            "reckoner",
            "reck-1",
            "implement a really complex feature with many words that goes on and on",
        );
        assert_eq!(
            branch,
            "reckoner/feat/reck-1-implement-a-really-complex-feature"
        );
    }
}
