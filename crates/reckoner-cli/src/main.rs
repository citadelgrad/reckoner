mod commands;

use std::path::PathBuf;

use clap::{Args, ArgGroup, Parser, Subcommand};
use reckoner_core::config::Config;
use reckoner_core::task::IntentSource;

/// Arguments for `reck task`.
///
/// Exactly one intent source must be provided. `--prd` is a supplementary flag
/// that is only valid alongside `--spec`.
#[derive(Args)]
#[command(group(
    ArgGroup::new("intent")
        .required(true)
        .args(["prompt", "spec", "epic"])
))]
struct TaskArgs {
    /// Repo name
    repo: String,

    /// Free-form prompt to pass directly to claude
    #[arg(long)]
    prompt: Option<String>,

    /// Path to a spec file; its contents are sent to claude as the prompt
    #[arg(long)]
    spec: Option<PathBuf>,

    /// Path to a PRD file supplying context (only valid with --spec)
    #[arg(long, requires = "spec")]
    prd: Option<PathBuf>,

    /// Epic description to use as the prompt
    #[arg(long)]
    epic: Option<String>,

    /// Use a specific .dot pipeline file (escape hatch for pre-built pipelines)
    #[arg(long)]
    pipeline: Option<String>,

    /// Skip PR creation (just run and collect logs)
    #[arg(long)]
    no_pr: bool,

    /// Keep the worktree after task completion
    #[arg(long)]
    keep_worktree: bool,
}

#[derive(Parser)]
#[command(
    name = "reck",
    version,
    about = "Reckoner — software factory wrapping PAS"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Register a git repo (bare treeless clone)
    Add {
        /// Git URL (SSH or HTTPS)
        url: String,
        /// Local working directory for post-PR branch checkout (optional)
        #[arg(long)]
        working_dir: Option<String>,
    },

    /// List registered repos
    List,

    /// Unregister a repo
    Remove {
        /// Repo name
        name: String,
    },

    /// Fetch latest changes for a repo
    Sync {
        /// Repo name
        name: String,
    },

    /// Run a task: provision container, run PAS, collect results
    Task(TaskArgs),

    /// Show task status
    Status {
        /// Task ID (omit for all active tasks)
        task_id: Option<String>,
    },

    /// Show preserved logs for a task
    Logs {
        /// Task ID
        task_id: String,

        /// Show only app logs
        #[arg(long)]
        app: bool,

        /// Show only lint results
        #[arg(long)]
        lint: bool,

        /// Filter log lines containing this pattern
        #[arg(long, short)]
        filter: Option<String>,
    },

    /// Run toolchain + architectural linters against a repo
    Lint {
        /// Repo name
        repo: String,
    },

    /// Manage scheduled background pipelines
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },

    /// Manage observability infrastructure (Loki + Grafana)
    Infra {
        #[command(subcommand)]
        action: InfraAction,
    },

    /// Open observability dashboard in browser
    Observe,

    /// Check system health
    Doctor,

    /// Show current configuration
    Config,

    /// Initialize Reckoner (create dirs, default config)
    Init,

    /// Manage registered repos
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
}

#[derive(Subcommand)]
enum ScheduleAction {
    /// Add a new scheduled pipeline
    Add {
        /// Schedule name (e.g., entropy-gc)
        #[arg(long)]
        name: String,

        /// Repo name
        #[arg(long)]
        repo: String,

        /// Path to .dot pipeline file
        #[arg(long)]
        pipeline: String,

        /// Cron expression (e.g., "0 3 * * *" for daily at 3am)
        #[arg(long)]
        cron: String,
    },

    /// List all schedules
    List,

    /// Remove a schedule
    Remove {
        /// Schedule name
        name: String,
    },

    /// Manually run a scheduled pipeline now
    Run {
        /// Schedule name
        name: String,

        /// Repo name
        #[arg(long)]
        repo: String,

        /// Path to .dot pipeline file
        #[arg(long)]
        pipeline: String,
    },
}

#[derive(Subcommand)]
enum InfraAction {
    /// Start observability stack (Loki + Grafana)
    Up,
    /// Stop observability stack
    Down,
    /// Check observability stack status
    Status,
}

#[derive(Subcommand)]
enum RepoAction {
    /// Set the local working directory for a registered repo (used for post-PR branch checkout)
    SetWorkingDir {
        /// Repo name
        name: String,
        /// Absolute path to the working directory
        path: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = Config::load(&Config::config_path())?;
    config.ensure_dirs()?;

    match cli.command {
        Commands::Add { url, working_dir } => {
            commands::repo::add(&url, working_dir.as_deref(), &config)?;
        }
        Commands::List => {
            commands::repo::list(&config)?;
        }
        Commands::Remove { name } => {
            commands::repo::remove(&name, &config)?;
        }
        Commands::Sync { name } => {
            commands::repo::sync(&name, &config)?;
        }
        Commands::Task(args) => {
            let intent = if let Some(p) = args.prompt {
                IntentSource::Prompt(p)
            } else if let Some(s) = args.spec {
                IntentSource::Spec {
                    spec: s,
                    prd: args.prd,
                }
            } else if let Some(e) = args.epic {
                IntentSource::Epic(e)
            } else {
                unreachable!("arg group guarantees one intent source")
            };
            commands::task::run(
                &args.repo,
                intent,
                args.pipeline.as_deref(),
                !args.no_pr,
                args.keep_worktree,
                &config,
            )
            .await?;
        }
        Commands::Status { task_id } => {
            if let Some(id) = task_id {
                commands::status::show_one(&id, &config)?;
            } else {
                commands::status::show_all(&config)?;
            }
        }
        Commands::Lint { repo } => {
            commands::lint::run(&repo, &config)?;
        }
        Commands::Logs {
            task_id,
            app,
            lint,
            filter,
        } => {
            if app || lint {
                // Show a specific log file, using hl if available
                let file = if app { "stdout.jsonl" } else { "linter.jsonl" };
                let path = config.general.logs_dir.join(&task_id).join(file);
                if path.exists() {
                    reckoner_core::infra::view_log_with_hl(&path, filter.as_deref())?;
                } else {
                    println!("No {} log found for task {}", file, task_id);
                }
            } else {
                // Show summary of all log files for this task
                let summary =
                    reckoner_core::logs::list_log_files(&config.general.logs_dir, &task_id)?;
                print!("{}", reckoner_core::logs::format_summary(&summary));
            }
        }
        Commands::Schedule { action } => match action {
            ScheduleAction::Add {
                name,
                repo,
                pipeline,
                cron,
            } => {
                commands::schedule::add(&name, &repo, &pipeline, &cron, &config)?;
            }
            ScheduleAction::List => {
                commands::schedule::list()?;
            }
            ScheduleAction::Remove { name } => {
                commands::schedule::remove(&name)?;
            }
            ScheduleAction::Run {
                name,
                repo,
                pipeline,
            } => {
                commands::schedule::run_now(&name, &repo, &pipeline, &config)?;
            }
        },
        Commands::Infra { action } => match action {
            InfraAction::Up => {
                reckoner_core::infra::infra_up()?;
            }
            InfraAction::Down => {
                reckoner_core::infra::infra_down()?;
            }
            InfraAction::Status => {
                let status = reckoner_core::infra::infra_status()?;
                println!("{}", status);
            }
        },
        Commands::Observe => {
            let url = "http://localhost:3148";
            println!("Opening Grafana at {}", url);
            let _ = std::process::Command::new("open").arg(url).status();
        }
        Commands::Doctor => {
            commands::doctor::run(&config)?;
        }
        Commands::Config => {
            let path = Config::config_path();
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                print!("{}", content);
            } else {
                println!("No config file at {}", path.display());
                println!("Using defaults. Run `reck init` to create one.");
            }
        }
        Commands::Repo { action } => match action {
            RepoAction::SetWorkingDir { name, path } => {
                commands::repo::set_working_dir(&name, &path, &config)?;
            }
        },
        Commands::Init => {
            let path = Config::config_path();
            if path.exists() {
                println!("Config already exists at {}", path.display());
            } else {
                let default = Config::default();
                let content = toml::to_string_pretty(&default)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &content)?;
                println!("Created config at {}", path.display());
            }
            config.ensure_dirs()?;
            println!("Directories ready at ~/.reckoner/");

            // Init database
            let _db = reckoner_core::db::Db::open(&config.general.db_path)?;
            println!("Database ready at {}", config.general.db_path.display());
        }
    }

    Ok(())
}
