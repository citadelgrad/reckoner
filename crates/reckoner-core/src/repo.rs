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
            base_branch,
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

/// Check if the worktree has any uncommitted changes.
pub fn has_changes(worktree_path: &Path) -> anyhow::Result<bool> {
    let output = git(worktree_path, &["status", "--porcelain"])?;
    Ok(!output.is_empty())
}

/// Stage all changes and commit in a worktree.
pub fn commit_all(worktree_path: &Path, message: &str, author: &str) -> anyhow::Result<()> {
    git(worktree_path, &["add", "-A"])?;

    // Check if there's anything to commit after staging
    let staged = git(worktree_path, &["diff", "--cached", "--stat"])?;
    if staged.is_empty() {
        tracing::info!("nothing to commit");
        return Ok(());
    }

    // Parse "Name <email>" from author string
    let commit_args = vec![
        "commit",
        "-m",
        message,
        "--author",
        author,
    ];
    git(worktree_path, &commit_args)?;
    tracing::info!("committed changes");
    Ok(())
}

/// Push a branch to origin.
pub fn push(worktree_path: &Path, branch_name: &str) -> anyhow::Result<()> {
    tracing::info!(branch = branch_name, "pushing to origin");
    git(worktree_path, &["push", "-u", "origin", branch_name])?;
    Ok(())
}

/// Create a PR using gh CLI. Returns the PR URL.
pub fn create_pr(
    worktree_path: &Path,
    title: &str,
    body: &str,
    base_branch: &str,
) -> anyhow::Result<String> {
    tracing::info!(title, base = base_branch, "creating PR");
    let output = Command::new("gh")
        .args([
            "pr", "create",
            "--title", title,
            "--body", body,
            "--base", base_branch,
            "--json", "url",
            "--jq", ".url",
        ])
        .current_dir(worktree_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr create failed: {}", stderr.trim());
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    tracing::info!(url, "PR created");
    Ok(url)
}

/// Get a short diffstat for the PR body.
pub fn diffstat(worktree_path: &Path, base_branch: &str) -> anyhow::Result<String> {
    // Diff against the base branch (origin/base)
    git(worktree_path, &["diff", "--stat", &format!("origin/{}", base_branch), "HEAD"])
        .or_else(|_| git(worktree_path, &["diff", "--stat", "HEAD~1", "HEAD"]))
        .unwrap_or_else(|_| "unable to compute diff".into());
    git(worktree_path, &["diff", "--stat", "HEAD~1", "HEAD"])
}

/// Build a structured PR body.
pub fn pr_body(task_id: &str, prompt: &str, diffstat: &str) -> String {
    format!(
        "## Summary\n\
         \n\
         {prompt}\n\
         \n\
         ## Changes\n\
         \n\
         ```\n\
         {diffstat}\n\
         ```\n\
         \n\
         ## Context\n\
         \n\
         - **Task**: `{task_id}`\n\
         - **Generated by**: Reckoner\n\
         \n\
         ---\n\
         <sub>This PR was automatically generated by Reckoner. Human review required.</sub>"
    )
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

    #[test]
    fn branch_name_handles_special_chars() {
        let branch = task_branch_name("reckoner", "reck-7", "fix bug #42: null pointer!");
        assert_eq!(branch, "reckoner/feat/reck-7-fix-bug-42-null-pointer");
    }

    #[test]
    fn pr_body_contains_required_sections() {
        let body = pr_body("reck-42", "add user auth", "README.md | 2 ++");
        assert!(body.contains("## Summary"));
        assert!(body.contains("add user auth"));
        assert!(body.contains("## Changes"));
        assert!(body.contains("README.md | 2 ++"));
        assert!(body.contains("reck-42"));
        assert!(body.contains("Reckoner"));
        assert!(body.contains("Human review required"));
    }

    #[test]
    fn pr_body_empty_diffstat() {
        let body = pr_body("reck-1", "fix a bug", "");
        assert!(body.contains("## Summary"));
        assert!(body.contains("fix a bug"));
    }

    #[test]
    fn has_changes_and_commit_in_temp_repo() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let repo_path = dir.path();

        // Init a real git repo
        git_in_cwd(&["init", &repo_path.to_string_lossy()]).unwrap();
        git(repo_path, &["config", "user.name", "Test"]).unwrap();
        git(repo_path, &["config", "user.email", "test@test.com"]).unwrap();

        // Create initial commit so HEAD exists
        std::fs::write(repo_path.join("init.txt"), "init").unwrap();
        git(repo_path, &["add", "-A"]).unwrap();
        git(repo_path, &["commit", "-m", "initial"]).unwrap();

        // No changes after commit
        assert!(!has_changes(repo_path).unwrap());

        // Create a file — now there are changes
        std::fs::write(repo_path.join("test.txt"), "hello").unwrap();
        assert!(has_changes(repo_path).unwrap());

        // Commit it
        commit_all(repo_path, "test commit", "Test <test@test.com>").unwrap();
        assert!(!has_changes(repo_path).unwrap());
    }
}
