use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::toolchain::{LanguageTools, ToolchainConfig};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub container: ContainerConfig,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub pas: PasConfig,
    #[serde(default)]
    pub linters: LinterConfig,
    #[serde(default)]
    pub toolchain: ToolchainGlobalConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LinterConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub fail_on_warning: bool,
    #[serde(default = "default_max_fix_iterations")]
    pub max_fix_iterations: u32,
    #[serde(default = "default_max_file_lines")]
    pub max_file_lines: u32,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ToolchainGlobalConfig {
    #[serde(default)]
    pub defaults: ToolchainConfig,
}

fn default_max_fix_iterations() -> u32 { 3 }
fn default_max_file_lines() -> u32 { 500 }

impl Default for LinterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fail_on_warning: false,
            max_fix_iterations: default_max_fix_iterations(),
            max_file_lines: default_max_file_lines(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GeneralConfig {
    #[serde(default = "default_repos_dir")]
    pub repos_dir: PathBuf,
    #[serde(default = "default_worktrees_dir")]
    pub worktrees_dir: PathBuf,
    #[serde(default = "default_logs_dir")]
    pub logs_dir: PathBuf,
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ContainerConfig {
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default = "default_base_image")]
    pub base_image: String,
    #[serde(default = "default_network")]
    pub network: String,
    #[serde(default = "default_memory")]
    pub default_memory: String,
    #[serde(default = "default_cpus")]
    pub default_cpus: u32,
    #[serde(default = "default_pids_limit")]
    pub pids_limit: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GitConfig {
    #[serde(default = "default_true")]
    pub auto_pr: bool,
    #[serde(default = "default_pr_prefix")]
    pub pr_prefix: String,
    #[serde(default = "default_commit_author")]
    pub commit_author: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PasConfig {
    #[serde(default = "default_pas_binary")]
    pub binary: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default = "default_budget")]
    pub default_max_budget_usd: f64,
    #[serde(default = "default_max_steps")]
    pub default_max_steps: u64,
}

fn reckoner_dir() -> PathBuf {
    dirs::home_dir()
        .expect("HOME not set")
        .join(".reckoner")
}

fn default_repos_dir() -> PathBuf { reckoner_dir().join("repos") }
fn default_worktrees_dir() -> PathBuf { reckoner_dir().join("worktrees") }
fn default_logs_dir() -> PathBuf { reckoner_dir().join("logs") }
fn default_db_path() -> PathBuf { reckoner_dir().join("reckoner.db") }
fn default_runtime() -> String { "orbstack".into() }
fn default_base_image() -> String { "reckoner-base:latest".into() }
fn default_network() -> String { "reckoner-net".into() }
fn default_memory() -> String { "4g".into() }
fn default_cpus() -> u32 { 4 }
fn default_pids_limit() -> u64 { 512 }
fn default_true() -> bool { true }
fn default_pr_prefix() -> String { "reckoner".into() }
fn default_commit_author() -> String { "Reckoner <reckoner@local>".into() }
fn default_pas_binary() -> String { "pas".into() }
fn default_model() -> String { "sonnet".into() }
fn default_budget() -> f64 { 10.0 }
fn default_max_steps() -> u64 { 200 }

impl Config {
    pub fn linters_enabled(&self) -> bool {
        self.linters.enabled
    }

    pub fn linter_max_lines(&self) -> u32 {
        self.linters.max_file_lines
    }

    pub fn toolchain_defaults(&self) -> &ToolchainConfig {
        &self.toolchain.defaults
    }
}

impl Default for Config {
    fn default() -> Self {
        // Default toolchain presets
        let mut tc_defaults = ToolchainConfig::new();
        tc_defaults.insert("python".into(), LanguageTools {
            lint: Some("ruff check --fix .".into()),
            format: Some("ruff format .".into()),
            typecheck: Some("ty check .".into()),
        });
        tc_defaults.insert("typescript".into(), LanguageTools {
            lint: Some("biome check --fix .".into()),
            format: Some("biome format --fix .".into()),
            typecheck: Some("biome check .".into()),
        });
        tc_defaults.insert("rust".into(), LanguageTools {
            lint: Some("cargo clippy --workspace -- -D warnings".into()),
            format: Some("cargo fmt --all".into()),
            typecheck: Some("cargo check --workspace".into()),
        });

        Self {
            general: GeneralConfig::default(),
            container: ContainerConfig::default(),
            git: GitConfig::default(),
            pas: PasConfig::default(),
            linters: LinterConfig::default(),
            toolchain: ToolchainGlobalConfig { defaults: tc_defaults },
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            repos_dir: default_repos_dir(),
            worktrees_dir: default_worktrees_dir(),
            logs_dir: default_logs_dir(),
            db_path: default_db_path(),
        }
    }
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            runtime: default_runtime(),
            base_image: default_base_image(),
            network: default_network(),
            default_memory: default_memory(),
            default_cpus: default_cpus(),
            pids_limit: default_pids_limit(),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            auto_pr: true,
            pr_prefix: default_pr_prefix(),
            commit_author: default_commit_author(),
        }
    }
}

impl Default for PasConfig {
    fn default() -> Self {
        Self {
            binary: default_pas_binary(),
            default_model: default_model(),
            default_max_budget_usd: default_budget(),
            default_max_steps: default_max_steps(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&contents)?)
        } else {
            Ok(Config::default())
        }
    }

    pub fn config_path() -> PathBuf {
        reckoner_dir().join("config.toml")
    }

    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.general.repos_dir)?;
        std::fs::create_dir_all(&self.general.worktrees_dir)?;
        std::fs::create_dir_all(&self.general.logs_dir)?;
        if let Some(parent) = self.general.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_values() {
        let cfg = Config::default();
        assert!(cfg.general.repos_dir.ends_with(".reckoner/repos"));
        assert_eq!(cfg.container.default_cpus, 4);
        assert_eq!(cfg.pas.default_model, "sonnet");
    }

    #[test]
    fn parses_minimal_toml() {
        let toml_str = r#"
[pas]
default_model = "opus"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.pas.default_model, "opus");
        assert_eq!(cfg.container.default_cpus, 4); // default preserved
    }

    #[test]
    fn default_linter_config() {
        let cfg = Config::default();
        assert!(cfg.linters_enabled());
        assert_eq!(cfg.linter_max_lines(), 500);
        assert_eq!(cfg.linters.max_fix_iterations, 3);
        assert!(!cfg.linters.fail_on_warning);
    }

    #[test]
    fn default_toolchain_has_python_rust_typescript() {
        let cfg = Config::default();
        let defaults = cfg.toolchain_defaults();
        assert!(defaults.contains_key("python"));
        assert!(defaults.contains_key("rust"));
        assert!(defaults.contains_key("typescript"));

        let py = &defaults["python"];
        assert!(py.lint.as_ref().unwrap().contains("ruff"));
        assert!(py.typecheck.as_ref().unwrap().contains("ty"));

        let ts = &defaults["typescript"];
        assert!(ts.lint.as_ref().unwrap().contains("biome"));
    }

    #[test]
    fn parses_linter_overrides() {
        let toml_str = r#"
[linters]
enabled = false
max_file_lines = 1000
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(!cfg.linters_enabled());
        assert_eq!(cfg.linter_max_lines(), 1000);
    }

    #[test]
    fn load_returns_defaults_for_missing_file() {
        let path = std::path::Path::new("/nonexistent/config.toml");
        let cfg = Config::load(path).unwrap();
        assert_eq!(cfg.pas.default_model, "sonnet");
    }

    #[test]
    fn load_reads_file() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[pas]\ndefault_model = \"haiku\"\n").unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.pas.default_model, "haiku");
    }

    #[test]
    fn ensure_dirs_creates_directories() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let cfg = Config {
            general: GeneralConfig {
                repos_dir: dir.path().join("repos"),
                worktrees_dir: dir.path().join("wt"),
                logs_dir: dir.path().join("logs"),
                db_path: dir.path().join("db/reckoner.db"),
            },
            ..Config::default()
        };
        cfg.ensure_dirs().unwrap();
        assert!(dir.path().join("repos").exists());
        assert!(dir.path().join("wt").exists());
        assert!(dir.path().join("logs").exists());
        assert!(dir.path().join("db").exists());
    }
}
