# PRD: VPS-Hosted Container Execution

## Problem

Reckoner currently runs as a local macOS CLI tool. Tasks execute on the host machine using the user's Claude subscription via Keychain auth. This limits Reckoner to a single operator on a single machine — it cannot serve a team, run unattended on a server, or provide remote access to task management.

The container runtime (`container.rs`), Dockerfile, and entrypoint script were built early but never wired into the task execution path. All task execution bypasses containers entirely, calling `claude`/`pas` directly on the host.

## Goals

1. **Container-first execution** — every task runs inside a Docker container with security isolation, not on the host
2. **VPS deployment** — Reckoner runs as a service on a Linux VPS, accessible remotely
3. **API access** — tasks can be triggered, monitored, and queried over HTTP
4. **Multi-user** — multiple users can submit tasks with isolated credentials and permissions
5. **Linux scheduling** — background pipelines work on Linux (replace macOS launchd)

## Non-Goals

- Kubernetes orchestration (Docker Compose is sufficient for now)
- Web UI (API-first; CLI and curl are the clients)
- Multi-VPS clustering or horizontal scaling
- Custom LLM provider support beyond Claude/PAS

## Users

- **Solo developer** — runs Reckoner on a VPS to process tasks against their repos while away from their machine
- **Small team** — 2-5 developers sharing a Reckoner instance, each with their own API key, submitting tasks against shared or individual repos
- **CI/CD integration** — GitHub Actions or similar triggers Reckoner tasks via API on push/PR events

## Requirements

### P0 — Must Have

1. **Container task execution** — `reck task` runs Claude/PAS inside the existing Docker container image, not on the host. The container gets the worktree mounted at `/workspace`, logs at `/var/log/reckoner`, and API keys injected via environment variables.

2. **API server** — HTTP API that supports:
   - `POST /api/tasks` — create and run a task (repo, prompt, optional pipeline)
   - `GET /api/tasks` — list tasks (filterable by status, repo)
   - `GET /api/tasks/:id` — get task detail (status, logs summary, PR URL, cost)
   - `DELETE /api/tasks/:id` — cancel a running task
   - `GET /api/repos` — list registered repos
   - `POST /api/repos` — register a repo
   - `GET /health` — health check

3. **API authentication** — API key-based auth. Each key is scoped to a user. Keys are stored hashed in SQLite. All API endpoints require a valid key except `/health`.

4. **Secret management** — `ANTHROPIC_API_KEY` and `GH_TOKEN` are stored per-user in the database (encrypted at rest) and injected into task containers at runtime. No secrets in config files or environment.

5. **Linux scheduling** — replace launchd with an in-process scheduler (tokio-cron or similar). Schedule definitions stay in SQLite. The scheduler runs inside the Reckoner server process.

6. **Deployment compose** — a single `docker-compose.yml` that runs:
   - Reckoner API server
   - Loki (log aggregation)
   - Grafana (dashboards)
   - Shared Docker socket for spawning task containers

### P1 — Should Have

7. **Log streaming** — `GET /api/tasks/:id/logs` streams logs via SSE (Server-Sent Events) while a task is running, returns complete JSONL after completion.

8. **Webhook receiver** — `POST /api/webhooks/github` receives GitHub push/PR events and triggers tasks based on configured rules per repo.

9. **Rate limiting** — per-user rate limits on task creation (e.g., max 5 concurrent tasks per user).

10. **CLI remote mode** — `reck` CLI can target a remote Reckoner API server via `--server <url>` flag, using an API key from the config file or environment.

### P2 — Nice to Have

11. **Task queuing** — when all container slots are full, tasks queue with FIFO ordering and priority support.

12. **Cost alerts** — per-user budget limits with notifications when approaching threshold.

13. **Repo access control** — users can only submit tasks against repos they have access to.

## Success Metrics

- A task triggered via `curl POST /api/tasks` executes inside a container, produces a PR, and returns the PR URL — without any local machine involvement
- Two users can submit tasks concurrently against the same repo without interference
- The Reckoner server restarts cleanly after a crash, resuming awareness of in-progress tasks
- Scheduled pipelines fire on time on a Linux VPS

## Risks

- **Docker-in-Docker complexity** — the Reckoner server itself may run in a container but needs to spawn sibling containers. Docker socket mounting is the simplest approach but has security implications. Mitigated by container security hardening already in `container.rs`.
- **Claude API cost control** — without the subscription model, API key usage needs budget tracking. The `max_budget_usd` config already exists but needs enforcement at the API key level.
- **SQLite under concurrency** — WAL mode handles concurrent reads well, but concurrent writes from multiple task completions could bottleneck. Monitor and migrate to Postgres if needed.
