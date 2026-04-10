# Reckoner

A software factory that wraps [PAS](https://github.com/citadelgrad/pascals-discrete-attractor) (Pascal's Discrete Attractor). Accepts git repo URLs, creates isolated worktrees per task, runs AI pipelines against them, and collects structured logs.

**Name origin:** Pascal's mechanical calculator was called "the Reckoner." Reckoner is the machine; PAS is the engine inside it.

<p align="center">
  <img src="docs/task-lifecycle.svg" alt="How Reckoner Works" width="800"/>
</p>

### How task isolation works

Reckoner never checks out code into the bare clone. Each task gets its own git worktree with a dedicated branch, sharing the bare clone's object store. A 2GB repo is stored once — not duplicated per task. Multiple tasks run concurrently against the same repo without conflicts, and worktrees are torn down after completion while logs are preserved forever.

<p align="center">
  <img src="docs/task-isolation.svg" alt="Concurrent Task Isolation" width="800"/>
</p>

## Quick Start

```sh
cargo install --path crates/reckoner-cli

reck init                                          # Create ~/.reckoner/ dirs + config + db
reck add git@github.com:user/my-project.git        # Register a repo (bare treeless clone)
reck task my-project "add user authentication"     # Run Claude against an isolated worktree
reck status                                        # Show active tasks
reck logs <task-id>                                # View preserved logs
```

## Commands

```
reck init                    Initialize (~/.reckoner/)
reck add <git-url>           Register a repo
reck list                    List registered repos
reck remove <name>           Unregister a repo
reck sync <name>             Fetch latest changes
reck task <repo> "<prompt>"  Run a task (fetch → worktree → claude → lint → PR → cleanup)
reck task <repo> "<prompt>" --pipeline <file.dot>   Use a PAS pipeline
reck lint <repo>             Run toolchain + architectural linters
reck status                  All active tasks
reck status <task-id>        Detailed task view
reck logs <task-id>          View preserved logs
reck doctor                  Health checks (git, gh, pas, docker, API keys)
reck config                  Show configuration
reck schedule add/list/remove/run   Manage background pipelines (launchd)
reck infra up/down/status    Manage observability stack (Loki + Grafana)
reck observe                 Open Grafana dashboard
```

## Architecture

Two crates:

| Crate | What |
|-------|------|
| **reckoner-core** | Config, SQLite (rusqlite), repo management (git CLI), container lifecycle (bollard), task orchestration, lint-fix loop, scheduling |
| **reckoner-cli** | clap CLI binary (`reck`) |

Key decisions:
- **rusqlite** (sync) over sqlx — matches SQLite's synchronous reality, faster CLI startup
- **Shell out to `git`/`gh`** — not git2/gitoxide; auth is free, no C deps
- **Bare clones + worktrees** — shared object store, concurrent task isolation
- **Structured JSONL logs** on disk — zero infrastructure, queryable with [`hl`](https://github.com/pamburus/hl)
- **Host execution** for Claude/PAS — uses your Claude subscription (macOS Keychain auth), no API key needed
- **Pluggable toolchain** — auto-detects Python (ruff/ty), TypeScript (biome), Rust (clippy/fmt) and runs lint-fix loops

## Prerequisites

- [PAS](https://github.com/citadelgrad/pascals-discrete-attractor) (`pas` on PATH)
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (`claude` on PATH, logged in)
- Git, GitHub CLI (`gh`)
- [OrbStack](https://orbstack.dev/) or Docker (for container-based linting/testing)

## Configuration

Config lives at `~/.reckoner/config.toml`. Run `reck init` to create defaults.

```toml
[general]
repos_dir = "~/.reckoner/repos"
worktrees_dir = "~/.reckoner/worktrees"
logs_dir = "~/.reckoner/logs"

[pas]
binary = "pas"
default_model = "sonnet"
default_max_budget_usd = 10.0

[git]
auto_pr = true
pr_prefix = "reckoner"

[linters]
enabled = true
max_fix_iterations = 3
max_file_lines = 500
```

## State

- **Database:** `~/.reckoner/reckoner.db` (SQLite, WAL mode, `user_version` migrations)
- **Repos:** `~/.reckoner/repos/<name>.git` (bare clones)
- **Worktrees:** `~/.reckoner/worktrees/` (temporary, per-task)
- **Logs:** `~/.reckoner/logs/<task-id>/` (preserved after task completion)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
