use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;

static MIGRATIONS: &[M<'static>] = &[
    M::up(
        "CREATE TABLE repos (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            url            TEXT NOT NULL UNIQUE,
            name           TEXT NOT NULL,
            local_path     TEXT NOT NULL,
            default_branch TEXT NOT NULL DEFAULT 'main',
            last_synced    TEXT,
            created_at     TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    ),
    M::up(
        "CREATE TABLE schedules (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_id       INTEGER NOT NULL REFERENCES repos(id),
            name          TEXT NOT NULL,
            pipeline_path TEXT NOT NULL,
            cron_expr     TEXT NOT NULL,
            enabled       INTEGER NOT NULL DEFAULT 1,
            last_run      TEXT,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    ),
    M::up(
        "CREATE TABLE tasks (
            id              TEXT PRIMARY KEY,
            repo_id         INTEGER NOT NULL REFERENCES repos(id),
            schedule_id     INTEGER REFERENCES schedules(id),
            prompt          TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending','provisioning','running',
                                  'linting','pr_open','done','failed')),
            container_id    TEXT,
            branch_name     TEXT,
            pr_url          TEXT,
            pipeline_path   TEXT,
            total_cost_usd  REAL DEFAULT 0.0,
            attempt_count   INTEGER DEFAULT 1,
            failed_stage    TEXT,
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            started_at      TEXT,
            completed_at    TEXT
        )",
    ),
    M::up(
        "CREATE TABLE task_transitions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id     TEXT NOT NULL REFERENCES tasks(id),
            from_status TEXT NOT NULL,
            to_status   TEXT NOT NULL,
            timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
            detail      TEXT
        )",
    ),
    M::up(
        "CREATE TABLE runs (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id       TEXT NOT NULL REFERENCES tasks(id),
            pipeline_path TEXT NOT NULL,
            status        TEXT NOT NULL,
            cost_usd      REAL DEFAULT 0.0,
            duration_secs INTEGER,
            logs_path     TEXT NOT NULL,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    ),
    M::up(
        "CREATE TABLE lint_results (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            repo_id       INTEGER NOT NULL REFERENCES repos(id),
            task_id       TEXT REFERENCES tasks(id),
            lint_run_id   TEXT NOT NULL,
            rule_name     TEXT NOT NULL,
            status        TEXT NOT NULL,
            message       TEXT,
            remediation   TEXT,
            file_path     TEXT,
            line_number   INTEGER,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    ),
    M::up(
        "CREATE INDEX idx_tasks_status ON tasks(status);
         CREATE INDEX idx_tasks_repo_id ON tasks(repo_id);
         CREATE INDEX idx_tasks_schedule_id ON tasks(schedule_id);
         CREATE INDEX idx_runs_task_id ON runs(task_id);
         CREATE INDEX idx_lint_results_repo_task ON lint_results(repo_id, task_id);
         CREATE INDEX idx_schedules_enabled ON schedules(enabled)",
    ),
];

fn configure_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "cache_size", -64000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut conn = Connection::open(path)?;
        configure_pragmas(&conn)?;

        let migrations = Migrations::new(MIGRATIONS.to_vec());
        migrations.to_latest(&mut conn)?;

        // Set file permissions to 0600 on unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // ── Repo operations ──────────────────────────────────────────────

    pub fn insert_repo(
        &self,
        url: &str,
        name: &str,
        local_path: &str,
        default_branch: &str,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO repos (url, name, local_path, default_branch) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![url, name, local_path, default_branch],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_repo_by_name(&self, name: &str) -> anyhow::Result<Option<Repo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, name, local_path, default_branch, last_synced, created_at
             FROM repos WHERE name = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![name], |row| {
            Ok(Repo {
                id: row.get(0)?,
                url: row.get(1)?,
                name: row.get(2)?,
                local_path: row.get(3)?,
                default_branch: row.get(4)?,
                last_synced: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(Ok(repo)) => Ok(Some(repo)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_repos(&self) -> anyhow::Result<Vec<Repo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, name, local_path, default_branch, last_synced, created_at
             FROM repos ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Repo {
                id: row.get(0)?,
                url: row.get(1)?,
                name: row.get(2)?,
                local_path: row.get(3)?,
                default_branch: row.get(4)?,
                last_synced: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn remove_repo(&self, name: &str) -> anyhow::Result<bool> {
        let changed = self.conn.execute("DELETE FROM repos WHERE name = ?1", [name])?;
        Ok(changed > 0)
    }

    pub fn update_repo_synced(&self, repo_id: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE repos SET last_synced = datetime('now') WHERE id = ?1",
            [repo_id],
        )?;
        Ok(())
    }

    // ── Task operations ──────────────────────────────────────────────

    pub fn insert_task(
        &self,
        id: &str,
        repo_id: i64,
        prompt: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO tasks (id, repo_id, prompt) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, repo_id, prompt],
        )?;
        Ok(())
    }

    pub fn transition_task(
        &self,
        task_id: &str,
        from: &str,
        to: &str,
        detail: Option<&str>,
    ) -> anyhow::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE tasks SET status = ?1,
                started_at = CASE WHEN ?1 = 'provisioning' THEN datetime('now') ELSE started_at END,
                completed_at = CASE WHEN ?1 IN ('done', 'failed') THEN datetime('now') ELSE completed_at END
             WHERE id = ?2 AND status = ?3",
            rusqlite::params![to, task_id, from],
        )?;

        if changed > 0 {
            self.conn.execute(
                "INSERT INTO task_transitions (task_id, from_status, to_status, detail)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![task_id, from, to, detail],
            )?;
        }

        Ok(changed > 0)
    }

    pub fn set_task_container(&self, task_id: &str, container_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET container_id = ?1 WHERE id = ?2",
            rusqlite::params![container_id, task_id],
        )?;
        Ok(())
    }

    pub fn set_task_branch(&self, task_id: &str, branch: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET branch_name = ?1 WHERE id = ?2",
            rusqlite::params![branch, task_id],
        )?;
        Ok(())
    }

    pub fn set_task_pr(&self, task_id: &str, pr_url: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET pr_url = ?1 WHERE id = ?2",
            rusqlite::params![pr_url, task_id],
        )?;
        Ok(())
    }

    pub fn set_task_error(
        &self,
        task_id: &str,
        stage: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE tasks SET failed_stage = ?1, error_message = ?2 WHERE id = ?3",
            rusqlite::params![stage, message, task_id],
        )?;
        Ok(())
    }

    pub fn get_task(&self, task_id: &str) -> anyhow::Result<Option<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, prompt, status, container_id, branch_name,
                    pr_url, pipeline_path, total_cost_usd, attempt_count,
                    failed_stage, error_message, created_at, started_at, completed_at
             FROM tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![task_id], map_task)?;
        match rows.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_active_tasks(&self) -> anyhow::Result<Vec<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, repo_id, prompt, status, container_id, branch_name,
                    pr_url, pipeline_path, total_cost_usd, attempt_count,
                    failed_stage, error_message, created_at, started_at, completed_at
             FROM tasks WHERE status NOT IN ('done', 'failed')
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], map_task)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ── Run operations ───────────────────────────────────────────────

    pub fn insert_run(
        &self,
        task_id: &str,
        pipeline_path: &str,
        logs_path: &str,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO runs (task_id, pipeline_path, status, logs_path) VALUES (?1, ?2, 'running', ?3)",
            rusqlite::params![task_id, pipeline_path, logs_path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_run(
        &self,
        run_id: i64,
        status: &str,
        cost_usd: f64,
        duration_secs: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE runs SET status = ?1, cost_usd = ?2, duration_secs = ?3 WHERE id = ?4",
            rusqlite::params![status, cost_usd, duration_secs, run_id],
        )?;
        Ok(())
    }
}

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        prompt: row.get(2)?,
        status: row.get(3)?,
        container_id: row.get(4)?,
        branch_name: row.get(5)?,
        pr_url: row.get(6)?,
        pipeline_path: row.get(7)?,
        total_cost_usd: row.get(8)?,
        attempt_count: row.get(9)?,
        failed_stage: row.get(10)?,
        error_message: row.get(11)?,
        created_at: row.get(12)?,
        started_at: row.get(13)?,
        completed_at: row.get(14)?,
    })
}

#[derive(Debug, Clone)]
pub struct Repo {
    pub id: i64,
    pub url: String,
    pub name: String,
    pub local_path: String,
    pub default_branch: String,
    pub last_synced: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub repo_id: i64,
    pub prompt: String,
    pub status: String,
    pub container_id: Option<String>,
    pub branch_name: Option<String>,
    pub pr_url: Option<String>,
    pub pipeline_path: Option<String>,
    pub total_cost_usd: f64,
    pub attempt_count: i64,
    pub failed_stage: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn temp_db() -> (TempDir, Db) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Db::open(&db_path).unwrap();
        (dir, db)
    }

    #[test]
    fn migrations_run_and_insert_repo() {
        let (_dir, db) = temp_db();
        let id = db
            .insert_repo("git@github.com:user/repo.git", "repo", "/tmp/repo", "main")
            .unwrap();
        assert!(id > 0);

        let repos = db.list_repos().unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "repo");
    }

    #[test]
    fn task_state_transitions() {
        let (_dir, db) = temp_db();
        db.insert_repo("git@github.com:u/r.git", "r", "/tmp/r", "main")
            .unwrap();
        db.insert_task("reck-001", 1, "fix the bug").unwrap();

        assert!(db.transition_task("reck-001", "pending", "provisioning", None).unwrap());
        assert!(db.transition_task("reck-001", "provisioning", "running", None).unwrap());

        // Wrong from-state should fail
        assert!(!db.transition_task("reck-001", "pending", "done", None).unwrap());

        let task = db.get_task("reck-001").unwrap().unwrap();
        assert_eq!(task.status, "running");
        assert!(task.started_at.is_some());
    }
}
