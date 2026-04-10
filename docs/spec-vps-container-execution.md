# SPEC: VPS-Hosted Container Execution

## Overview

This spec covers the technical changes to transform Reckoner from a local macOS CLI into a VPS-hosted service with container-based task execution, an HTTP API, multi-user auth, and Linux scheduling.

## 1. Container Task Execution

### Current State

`task.rs:run_on_host()` (line 193) shells out to `claude`/`pas` directly on the host:

```rust
fn run_on_host(config, prompt, pipeline, worktree_path, logs_path) -> Result<i32>
```

`container.rs` has a complete Docker runtime via bollard but is never called. The `tasks` table has a `container_id` column that is never populated.

### Changes

**Replace `run_on_host()` with `run_in_container()`** in `task.rs`:

```rust
async fn run_in_container(
    runtime: &DockerRuntime,
    config: &Config,
    prompt: &str,
    pipeline: Option<&str>,
    worktree_path: &Path,
    logs_path: &Path,
    secrets: &TaskSecrets,  // ANTHROPIC_API_KEY, GH_TOKEN
) -> anyhow::Result<RunResult>
```

Flow:
1. Call `runtime.create()` with worktree + logs bind mounts (already implemented)
2. Inject `ANTHROPIC_API_KEY` and `GH_TOKEN` as container env vars
3. Call `runtime.start()`
4. Call `runtime.run_command()` with the claude/pas command
5. Call `runtime.collect_logs()` to capture output
6. Call `runtime.stop()` + `runtime.remove()`
7. Store `container_id` in the tasks table (column exists, currently unused)

**Keep `run_on_host()` as a fallback** behind a config flag `execution_mode`:

```toml
[general]
execution_mode = "container"  # or "host" for local dev
```

**Existing container security** (already in `container.rs`):
- `cap_drop: ALL` — no Linux capabilities
- `security_opt: no-new-privileges`
- Memory/CPU/PID limits from config
- tmpfs for `/tmp` (512MB) and cache (1GB)
- Non-root user (UID 1001)

No changes needed to the security model.

### Container Image

The existing `infra/Dockerfile` is sufficient. It installs:
- git, gh, curl, jq
- Node.js + Claude Code CLI
- uv (Python)
- Non-root `agent` user

Add to the image:
- `pas` binary (COPY from build stage or host)
- Rust toolchain (for Rust repo linting — optional, per-language images later)

## 2. API Server

### Framework

Add `axum` as the HTTP framework. It's async, tower-compatible, and pairs well with the existing tokio runtime.

### New Crate

Create `crates/reckoner-server/` with:

```
crates/reckoner-server/
  src/
    main.rs          # Server entry point
    routes/
      mod.rs
      tasks.rs       # POST/GET/DELETE /api/tasks
      repos.rs       # GET/POST /api/repos
      logs.rs        # GET /api/tasks/:id/logs (SSE)
      webhooks.rs    # POST /api/webhooks/github
      health.rs      # GET /health
    auth.rs          # API key validation middleware
    state.rs         # Shared app state (Db, DockerRuntime, Config)
    error.rs         # Error types → HTTP responses
```

### Endpoints

```
GET    /health                         → 200 { status: "ok" }
POST   /api/tasks                      → 202 { task_id, status: "pending" }
GET    /api/tasks                      → 200 [{ id, repo, status, created_at, ... }]
GET    /api/tasks/:id                  → 200 { id, status, pr_url, cost, logs_summary }
DELETE /api/tasks/:id                  → 200 { status: "cancelled" }
GET    /api/tasks/:id/logs             → 200 text/event-stream (SSE) or application/jsonl
GET    /api/repos                      → 200 [{ name, url, branch, last_synced }]
POST   /api/repos   { url }           → 201 { name, branch }
DELETE /api/repos/:name                → 200 { removed: true }
POST   /api/webhooks/github            → 200 (async task creation)
```

### Task Submission

`POST /api/tasks` accepts:

```json
{
  "repo": "my-project",
  "prompt": "add user authentication",
  "pipeline": "auth-pipeline.dot",  // optional
  "no_pr": false                     // optional, default false
}
```

The server inserts the task into SQLite, spawns a tokio task for execution, and returns immediately with the task ID. The spawned task runs the full lifecycle (provision → execute → verify → ship → cleanup).

### Shared State

```rust
struct AppState {
    db: Arc<Db>,
    config: Arc<Config>,
    runtime: Arc<DockerRuntime>,
    scheduler: Arc<Scheduler>,
}
```

## 3. Authentication

### API Keys

Store in a new `api_keys` table:

```sql
CREATE TABLE api_keys (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,       -- first 8 chars for identification
    name TEXT NOT NULL,             -- "scott's laptop", "CI key"
    created_at TEXT DEFAULT (datetime('now')),
    last_used_at TEXT,
    revoked_at TEXT
);
```

Keys are generated as `reck_<32 random hex chars>`, stored as SHA-256 hash. The prefix (`reck_abcd1234`) is stored in plaintext for identification in logs.

### Users

New `users` table:

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    anthropic_key_enc TEXT,         -- encrypted ANTHROPIC_API_KEY
    gh_token_enc TEXT,              -- encrypted GH_TOKEN
    max_concurrent_tasks INTEGER DEFAULT 5,
    max_budget_usd REAL DEFAULT 50.0,
    created_at TEXT DEFAULT (datetime('now'))
);
```

### Secret Encryption

Use `aes-gcm` crate. The server master key is loaded from an environment variable (`RECKONER_MASTER_KEY`) or a file (`/etc/reckoner/master.key`). User secrets are encrypted at rest in SQLite, decrypted only when injecting into a task container.

### Auth Middleware

Axum middleware extracts the API key from the `Authorization: Bearer reck_...` header, hashes it, looks up the user, and injects `AuthUser` into the request extensions.

### Schema Changes

Add `user_id` foreign key to:
- `tasks` — who submitted the task
- `repos` — who registered the repo (or NULL for shared repos)

## 4. Linux Scheduling

### Current State

`schedule.rs` generates macOS launchd plist XML and calls `launchctl load/unload`. This is completely macOS-specific.

### Replacement

Replace with an in-process scheduler using the `tokio-cron-scheduler` crate.

```rust
pub struct Scheduler {
    sched: JobScheduler,
    db: Arc<Db>,
    config: Arc<Config>,
    runtime: Arc<DockerRuntime>,
}

impl Scheduler {
    pub async fn start(&self) -> Result<()>;
    pub async fn add(&self, schedule: &Schedule) -> Result<Uuid>;
    pub async fn remove(&self, schedule_id: Uuid) -> Result<()>;
    pub async fn list(&self) -> Result<Vec<Schedule>>;
}
```

Schedule definitions stay in the existing `schedules` table. On server startup, all enabled schedules are loaded and registered with the in-process scheduler. The scheduler runs inside the server process — no external daemon needed.

### API Endpoints

```
GET    /api/schedules                  → 200 [{ id, name, repo, cron, enabled }]
POST   /api/schedules                  → 201 { id, name }
DELETE /api/schedules/:id              → 200 { removed: true }
POST   /api/schedules/:id/run          → 202 { task_id }
```

### Backward Compatibility

Keep the CLI `reck schedule` commands but have them call the API when `--server` is set, or manage the in-process scheduler directly when running locally.

## 5. Deployment

### Docker Compose

```yaml
# docker-compose.yml (production)
services:
  reckoner:
    build: .
    ports:
      - "8741:8741"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock  # sibling containers
      - reckoner-data:/data                         # SQLite + repos + logs
    environment:
      - RECKONER_MASTER_KEY_FILE=/run/secrets/master_key
      - RECKONER_DB_PATH=/data/reckoner.db
      - RECKONER_REPOS_DIR=/data/repos
      - RECKONER_LOGS_DIR=/data/logs
    restart: unless-stopped

  loki:
    image: grafana/loki:3.4
    ports:
      - "127.0.0.1:3147:3100"
    volumes:
      - loki-data:/loki

  grafana:
    image: grafana/grafana:11.5
    ports:
      - "127.0.0.1:3148:3000"
    environment:
      - GF_AUTH_ANONYMOUS_ENABLED=true
    volumes:
      - grafana-data:/var/lib/grafana

volumes:
  reckoner-data:
  loki-data:
  grafana-data:
```

### Reckoner Server Dockerfile

```dockerfile
FROM rust:1.83-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin reck-server

FROM ubuntu:24.04
RUN apt-get update && apt-get install -y ca-certificates git && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/reck-server /usr/local/bin/
EXPOSE 8741
CMD ["reck-server"]
```

### Reverse Proxy

Caddy recommended for automatic TLS:

```
reckoner.example.com {
    reverse_proxy localhost:8741
}
```

## 6. Config Changes

### New Config Fields

```toml
[general]
execution_mode = "container"    # "container" or "host"
data_dir = "/data"              # base for repos, logs, db on VPS

[server]
bind = "0.0.0.0:8741"
master_key_file = "/run/secrets/master_key"

[container]
runtime = "docker"              # was "orbstack"
```

### Environment Variable Overrides

All config fields can be overridden via `RECKONER_*` env vars:
- `RECKONER_BIND=0.0.0.0:8741`
- `RECKONER_DB_PATH=/data/reckoner.db`
- `RECKONER_EXECUTION_MODE=container`
- `RECKONER_MASTER_KEY` or `RECKONER_MASTER_KEY_FILE`

## 7. Database Migrations

Add migrations 8-10:

**Migration 8** — users + api_keys tables:
```sql
CREATE TABLE users (...);
CREATE TABLE api_keys (...);
```

**Migration 9** — add user_id to tasks and repos:
```sql
ALTER TABLE tasks ADD COLUMN user_id INTEGER REFERENCES users(id);
ALTER TABLE repos ADD COLUMN user_id INTEGER REFERENCES users(id);
```

**Migration 10** — scheduling updates:
```sql
ALTER TABLE schedules ADD COLUMN user_id INTEGER REFERENCES users(id);
ALTER TABLE schedules ADD COLUMN job_id TEXT;  -- in-process scheduler UUID
```

Existing single-user data gets `user_id = NULL` (treated as the default/admin user).

## 8. Implementation Order

| Phase | Work | Depends On |
|-------|------|------------|
| **A** | Wire `container.rs` into `task.rs` — container execution path with config toggle | Nothing |
| **B** | Create `reckoner-server` crate with axum, health endpoint, task CRUD | Nothing |
| **C** | API key auth + users table + secret encryption | Phase B |
| **D** | Connect server to task execution (spawn tasks via API) | Phase A + B |
| **E** | Replace launchd scheduler with tokio-cron-scheduler | Phase B |
| **F** | Log streaming (SSE) + webhook receiver | Phase D |
| **G** | Deployment compose + Caddy + production hardening | Phase D |
| **H** | CLI remote mode (`--server` flag) | Phase C |

Phases A and B can be developed in parallel. The critical path is A → D → G.
