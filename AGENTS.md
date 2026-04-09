# Reckoner — Agent Instructions

## Build & Test

```bash
cargo build --release          # Build CLI binary (reck)
cargo test                     # Run all tests
cargo clippy --workspace       # Lint
cargo fmt --all -- --check     # Format check
```

The CLI binary is `reck`. Install with `cargo install --path crates/reckoner-cli`.

## Architecture

Two crates:
- `reckoner-core` — engine: db, config, repo management, container lifecycle, task orchestration
- `reckoner-cli` — clap CLI binary that calls into core

## Key Conventions

- SQLite via **rusqlite** (sync, bundled). NOT sqlx.
- Container orchestration via **bollard** (Docker API client)
- Git operations via **shelling out to `git` CLI** (not git2/gitoxide)
- GitHub PR operations via **shelling out to `gh` CLI**
- Config via **toml** crate + serde
- Async only for container/git/network ops. DB layer is sync.
- State lives at `~/.reckoner/` (db, repos, worktrees, logs, config)
