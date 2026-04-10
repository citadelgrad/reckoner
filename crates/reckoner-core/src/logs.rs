use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A structured log entry written by Reckoner tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub source: String, // "claude", "toolchain", "linter", "git"
    pub message: String,
    #[serde(default)]
    pub task_id: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

/// Summary of a task's log directory.
#[derive(Debug)]
pub struct LogSummary {
    pub task_id: String,
    pub files: Vec<LogFile>,
    pub total_bytes: u64,
}

#[derive(Debug)]
pub struct LogFile {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub line_count: usize,
}

/// List all log files for a task.
pub fn list_log_files(logs_dir: &Path, task_id: &str) -> anyhow::Result<LogSummary> {
    let task_dir = logs_dir.join(task_id);
    if !task_dir.exists() {
        anyhow::bail!("no logs found for task {}", task_id);
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;

    let entries = std::fs::read_dir(&task_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = std::fs::metadata(&path)?;
        let size = meta.len();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let line_count = if size > 0 {
            std::fs::read_to_string(&path)
                .map(|c| c.lines().count())
                .unwrap_or(0)
        } else {
            0
        };

        total_bytes += size;
        files.push(LogFile {
            name,
            path,
            size_bytes: size,
            line_count,
        });
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(LogSummary {
        task_id: task_id.into(),
        files,
        total_bytes,
    })
}

/// Read a specific log file, optionally filtering lines containing a pattern.
pub fn read_log_file(path: &Path, filter: Option<&str>) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<String> = match filter {
        Some(pattern) => content
            .lines()
            .filter(|l| l.contains(pattern))
            .map(|l| l.to_string())
            .collect(),
        None => content.lines().map(|l| l.to_string()).collect(),
    };
    Ok(lines)
}

/// Parse JSONL log entries from a file, skipping malformed lines.
pub fn parse_jsonl_entries(path: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let content = std::fs::read_to_string(path)?;
    let entries: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(entries)
}

/// Get a human-readable summary of all logs for a task.
pub fn format_summary(summary: &LogSummary) -> String {
    let mut out = format!("Task: {}\n", summary.task_id);
    out.push_str(&format!(
        "Total: {} files, {} bytes\n\n",
        summary.files.len(),
        summary.total_bytes
    ));

    for f in &summary.files {
        out.push_str(&format!(
            "  {:<25} {:>6} bytes  {:>4} lines\n",
            f.name, f.size_bytes, f.line_count
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_log_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("reck-test1");
        std::fs::create_dir_all(&task_dir).unwrap();
        (dir, task_dir)
    }

    // ── list_log_files ───────────────────────────────────────────────

    #[test]
    fn list_log_files_returns_error_for_missing_task() {
        let dir = TempDir::new().unwrap();
        let result = list_log_files(dir.path(), "reck-nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no logs found"));
    }

    #[test]
    fn list_log_files_returns_empty_for_empty_dir() {
        let (dir, _task_dir) = setup_log_dir();
        let summary = list_log_files(dir.path(), "reck-test1").unwrap();
        assert_eq!(summary.task_id, "reck-test1");
        assert!(summary.files.is_empty());
        assert_eq!(summary.total_bytes, 0);
    }

    #[test]
    fn list_log_files_counts_files_and_bytes() {
        let (dir, task_dir) = setup_log_dir();
        std::fs::write(task_dir.join("stdout.jsonl"), "line1\nline2\n").unwrap();
        std::fs::write(task_dir.join("stderr.log"), "error\n").unwrap();

        let summary = list_log_files(dir.path(), "reck-test1").unwrap();
        assert_eq!(summary.files.len(), 2);
        assert!(summary.total_bytes > 0);

        let names: Vec<&str> = summary.files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"stderr.log"));
        assert!(names.contains(&"stdout.jsonl"));
    }

    #[test]
    fn list_log_files_counts_lines() {
        let (dir, task_dir) = setup_log_dir();
        std::fs::write(task_dir.join("out.jsonl"), "a\nb\nc\n").unwrap();

        let summary = list_log_files(dir.path(), "reck-test1").unwrap();
        assert_eq!(summary.files[0].line_count, 3);
    }

    #[test]
    fn list_log_files_sorted_by_name() {
        let (dir, task_dir) = setup_log_dir();
        std::fs::write(task_dir.join("z.log"), "z").unwrap();
        std::fs::write(task_dir.join("a.log"), "a").unwrap();
        std::fs::write(task_dir.join("m.log"), "m").unwrap();

        let summary = list_log_files(dir.path(), "reck-test1").unwrap();
        let names: Vec<&str> = summary.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a.log", "m.log", "z.log"]);
    }

    // ── read_log_file ────────────────────────────────────────────────

    #[test]
    fn read_log_file_returns_all_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

        let lines = read_log_file(&path, None).unwrap();
        assert_eq!(lines, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn read_log_file_filters_by_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "info: ok\nerror: bad\ninfo: fine\n").unwrap();

        let lines = read_log_file(&path, Some("error")).unwrap();
        assert_eq!(lines, vec!["error: bad"]);
    }

    #[test]
    fn read_log_file_filter_no_matches() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "hello\nworld\n").unwrap();

        let lines = read_log_file(&path, Some("nonexistent")).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn read_log_file_returns_error_for_missing() {
        let result = read_log_file(Path::new("/nonexistent/file.log"), None);
        assert!(result.is_err());
    }

    // ── parse_jsonl_entries ──────────────────────────────────────────

    #[test]
    fn parse_jsonl_entries_parses_valid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n").unwrap();

        let entries = parse_jsonl_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["a"], 1);
        assert_eq!(entries[1]["b"], 2);
    }

    #[test]
    fn parse_jsonl_entries_skips_malformed_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mixed.jsonl");
        std::fs::write(&path, "{\"ok\":true}\nnot json\n{\"also\":\"ok\"}\n").unwrap();

        let entries = parse_jsonl_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn parse_jsonl_entries_handles_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.jsonl");
        std::fs::write(&path, "").unwrap();

        let entries = parse_jsonl_entries(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_jsonl_entries_skips_blank_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sparse.jsonl");
        std::fs::write(&path, "{\"a\":1}\n\n\n{\"b\":2}\n").unwrap();

        let entries = parse_jsonl_entries(&path).unwrap();
        assert_eq!(entries.len(), 2);
    }

    // ── format_summary ───────────────────────────────────────────────

    #[test]
    fn format_summary_includes_task_id_and_files() {
        let summary = LogSummary {
            task_id: "reck-42".into(),
            files: vec![
                LogFile {
                    name: "stdout.jsonl".into(),
                    path: PathBuf::from("/logs/stdout.jsonl"),
                    size_bytes: 1024,
                    line_count: 50,
                },
                LogFile {
                    name: "stderr.log".into(),
                    path: PathBuf::from("/logs/stderr.log"),
                    size_bytes: 256,
                    line_count: 5,
                },
            ],
            total_bytes: 1280,
        };

        let output = format_summary(&summary);
        assert!(output.contains("reck-42"));
        assert!(output.contains("2 files"));
        assert!(output.contains("1280 bytes"));
        assert!(output.contains("stdout.jsonl"));
        assert!(output.contains("stderr.log"));
    }

    // ── LogEntry serialization ───────────────────────────────────────

    #[test]
    fn log_entry_round_trips_json() {
        let entry = LogEntry {
            timestamp: "2026-04-10T00:00:00Z".into(),
            level: "info".into(),
            source: "claude".into(),
            message: "task started".into(),
            task_id: "reck-1".into(),
            data: serde_json::json!({"model": "sonnet"}),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: LogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source, "claude");
        assert_eq!(parsed.task_id, "reck-1");
    }

    #[test]
    fn log_entry_without_data_omits_field() {
        let entry = LogEntry {
            timestamp: "2026-04-10T00:00:00Z".into(),
            level: "info".into(),
            source: "git".into(),
            message: "pushed".into(),
            task_id: "reck-2".into(),
            data: serde_json::Value::Null,
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("\"data\""));
    }
}
