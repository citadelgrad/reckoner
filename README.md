# Reckoner

A software factory that wraps [PAS](https://github.com/citadelgrad/pascals-discrete-attractor) (Pascal's Discrete Attractor). Accepts git repo URLs, creates isolated worktrees per task, runs AI pipelines against them, and collects structured logs.

**Name origin:** Pascal's mechanical calculator was called "the Reckoner." Reckoner is the machine; PAS is the engine inside it.

## Quick Start

```sh
cargo install --path crates/reckoner-cli

reck init                                          # Create ~/.reckoner/ dirs + config + db
reck add git@github.com:user/my-project.git        # Register a repo (bare treeless clone)
reck task my-project "add user authentication"     # Run Claude against an isolated worktree
reck status                                        # Show active tasks
reck logs <task-id>                                # View preserved logs
```

## How It Works

1. **`reck add`** — bare treeless clone (`--filter=blob:none`) for fast initial fetch
2. **`reck task`** — fetches latest, creates a git worktree + branch, runs `claude -p` (or `pas run`) against the worktree, saves structured JSONL logs, cleans up
3. **`reck status`** / **`reck logs`** — task tracking with SQLite, logs preserved on disk after worktree teardown

Each task gets its own worktree and branch (`reckoner/feat/<id>-<slug>`), so multiple tasks can run concurrently against the same repo without conflicts. All worktrees share the bare clone's object store — a 2GB repo is stored once, not N times.

## Commands

```
reck add <git-url>           Register a repo
reck list                    List registered repos
reck remove <name>           Unregister a repo
reck sync <name>             Fetch latest changes
reck task <repo> "<prompt>"  Run a task (fetch → worktree → claude → logs → cleanup)
reck task <repo> "<prompt>" --pipeline <file.dot>   Use a PAS pipeline
reck status                  All active tasks
reck status <task-id>        Detailed task view
reck logs <task-id>          View preserved logs
reck doctor                  Health checks (git, gh, pas, docker, API keys)
reck config                  Show configuration
reck init                    Initialize (~/.reckoner/)
```

## Architecture

Two crates:

| Crate | What |
|-------|------|
| **reckoner-core** | Config, SQLite (rusqlite), repo management (git CLI), container lifecycle (bollard), task orchestration |
| **reckoner-cli** | clap CLI binary (`reck`) |

Key decisions:
- **rusqlite** (sync) over sqlx — matches SQLite's synchronous reality, faster CLI startup
- **Shell out to `git`/`gh`** — not git2/gitoxide; auth is free, no C deps
- **Bare clones + worktrees** — shared object store, concurrent task isolation
- **Structured JSONL logs** on disk — zero infrastructure, queryable with [`hl`](https://github.com/pamburus/hl)
- **Host execution** for Claude/PAS — uses your Claude subscription (macOS Keychain auth), no API key needed

## Prerequisites

- [PAS](https://github.com/citadelgrad/pascals-discrete-attractor) (`pas` on PATH)
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (`claude` on PATH, logged in)
- Git, GitHub CLI (`gh`)
- [OrbStack](https://orbstack.dev/) or Docker (for future container-based linting/testing)

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
```

## State

- **Database:** `~/.reckoner/reckoner.db` (SQLite, WAL mode, `user_version` migrations)
- **Repos:** `~/.reckoner/repos/<name>.git` (bare clones)
- **Worktrees:** `~/.reckoner/worktrees/` (temporary, per-task)
- **Logs:** `~/.reckoner/logs/<task-id>/` (preserved after task completion)

## Roadmap

- [x] Phase 1-3: Foundation, repo management, task runner
- [ ] Phase 4: Git + PR integration (auto-commit, push, `gh pr create`)
- [ ] Phase 5: Pluggable toolchain (ruff/ty, biome, clippy) + architectural linters
- [ ] Phase 6: Observability (`hl` integration, optional Grafana)
- [ ] Phase 7: Background agents (launchd scheduling, entropy GC, doc gardening)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
