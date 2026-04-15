use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A launchd plist for a scheduled Reckoner pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LaunchAgent {
    pub label: String,
    pub program_arguments: Vec<String>,
    pub start_calendar_interval: CalendarInterval,
    pub standard_out_path: String,
    pub standard_error_path: String,
    pub process_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_priority_io: Option<bool>,
    pub environment_variables: HashMap<String, String>,
    pub working_directory: String,
}

/// Calendar interval for StartCalendarInterval.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct CalendarInterval {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hour: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minute: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekday: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<u32>,
}

/// Parse a simple cron expression into a CalendarInterval.
/// Fields: minute hour day month weekday (standard cron order).
/// Supports: "M H * * *" (daily), "M H * * D" (weekly), "M H D * *" (monthly).
pub fn parse_cron(expr: &str) -> anyhow::Result<CalendarInterval> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        anyhow::bail!(
            "cron expression must have 5 fields (minute hour day month weekday), got {}",
            parts.len()
        );
    }

    let minute = parse_cron_field(parts[0])?;
    let hour = parse_cron_field(parts[1])?;
    let day = parse_cron_field(parts[2])?;
    let month = parse_cron_field(parts[3])?;
    let weekday = parse_cron_field(parts[4])?;

    Ok(CalendarInterval {
        minute,
        hour,
        day,
        month,
        weekday,
    })
}

fn parse_cron_field(s: &str) -> anyhow::Result<Option<u32>> {
    if s == "*" {
        Ok(None)
    } else {
        let val: u32 = s
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid cron field: '{}'", s))?;
        Ok(Some(val))
    }
}

/// Build a LaunchAgent plist for a scheduled reck task.
pub fn build_plist(
    name: &str,
    reck_binary: &str,
    repo_name: &str,
    pipeline: &str,
    cron_expr: &str,
    logs_dir: &Path,
) -> anyhow::Result<LaunchAgent> {
    let interval = parse_cron(cron_expr)?;
    let label = format!("com.reckoner.{}", name);
    let log_dir = logs_dir.join("schedules");

    let mut env = HashMap::new();
    env.insert(
        "PATH".into(),
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".into(),
    );
    env.insert("RECKONER_LOG".into(), "info".into());

    Ok(LaunchAgent {
        label,
        program_arguments: vec![
            reck_binary.into(),
            "task".into(),
            repo_name.into(),
            format!("scheduled: {}", name),
            "--pipeline".into(),
            pipeline.into(),
            "--no-pr".into(),
        ],
        start_calendar_interval: interval,
        standard_out_path: log_dir
            .join(format!("{}.stdout.log", name))
            .to_string_lossy()
            .into(),
        standard_error_path: log_dir
            .join(format!("{}.stderr.log", name))
            .to_string_lossy()
            .into(),
        process_type: "Background".into(),
        low_priority_io: Some(true),
        environment_variables: env,
        working_directory: dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .to_string_lossy()
            .into(),
    })
}

/// Write a plist to the LaunchAgents directory.
pub fn write_plist(agent: &LaunchAgent) -> anyhow::Result<PathBuf> {
    let launch_agents_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("HOME not set"))?
        .join("Library/LaunchAgents");
    std::fs::create_dir_all(&launch_agents_dir)?;

    let filename = format!("{}.plist", agent.label);
    let path = launch_agents_dir.join(&filename);

    plist::to_file_xml(&path, agent)?;
    tracing::info!(path = %path.display(), label = %agent.label, "wrote plist");

    Ok(path)
}

/// Load/unload a plist via launchctl.
pub fn launchctl_load(plist_path: &Path) -> anyhow::Result<()> {
    let output = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl load failed: {}", stderr.trim());
    }
    Ok(())
}

pub fn launchctl_unload(plist_path: &Path) -> anyhow::Result<()> {
    let output = std::process::Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl unload failed: {}", stderr.trim());
    }
    Ok(())
}

/// Remove a plist file and unload it from launchctl.
pub fn remove_schedule(name: &str) -> anyhow::Result<()> {
    let launch_agents_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("HOME not set"))?
        .join("Library/LaunchAgents");
    let label = format!("com.reckoner.{}", name);
    let path = launch_agents_dir.join(format!("{}.plist", label));

    if path.exists() {
        let _ = launchctl_unload(&path);
        std::fs::remove_file(&path)?;
        tracing::info!(label, "removed schedule");
    } else {
        anyhow::bail!("no schedule found for '{}'", name);
    }

    Ok(())
}

/// List all Reckoner schedules by scanning LaunchAgents for com.reckoner.* plists.
pub fn list_schedules() -> anyhow::Result<Vec<(String, PathBuf)>> {
    let launch_agents_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("HOME not set"))?
        .join("Library/LaunchAgents");

    if !launch_agents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    for entry in std::fs::read_dir(&launch_agents_dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("com.reckoner.") && name.ends_with(".plist") {
            let label = name
                .strip_prefix("com.reckoner.")
                .and_then(|s| s.strip_suffix(".plist"))
                .unwrap_or(&name)
                .to_string();
            results.push((label, entry.path()));
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── parse_cron ───────────────────────────────────────────────────

    #[test]
    fn parse_cron_daily_3am() {
        let ci = parse_cron("0 3 * * *").unwrap();
        assert_eq!(ci.minute, Some(0));
        assert_eq!(ci.hour, Some(3));
        assert!(ci.day.is_none());
        assert!(ci.month.is_none());
        assert!(ci.weekday.is_none());
    }

    #[test]
    fn parse_cron_weekly_sunday_midnight() {
        let ci = parse_cron("0 0 * * 0").unwrap();
        assert_eq!(ci.minute, Some(0));
        assert_eq!(ci.hour, Some(0));
        assert_eq!(ci.weekday, Some(0));
    }

    #[test]
    fn parse_cron_monthly_first_at_noon() {
        let ci = parse_cron("0 12 1 * *").unwrap();
        assert_eq!(ci.hour, Some(12));
        assert_eq!(ci.day, Some(1));
    }

    #[test]
    fn parse_cron_rejects_wrong_field_count() {
        assert!(parse_cron("0 3 *").is_err());
        assert!(parse_cron("0 3 * * * *").is_err());
    }

    #[test]
    fn parse_cron_rejects_invalid_number() {
        assert!(parse_cron("abc 3 * * *").is_err());
    }

    #[test]
    fn parse_cron_all_wildcards() {
        let ci = parse_cron("* * * * *").unwrap();
        assert!(ci.minute.is_none());
        assert!(ci.hour.is_none());
        assert!(ci.day.is_none());
        assert!(ci.month.is_none());
        assert!(ci.weekday.is_none());
    }

    // ── build_plist ──────────────────────────────────────────────────

    #[test]
    fn build_plist_creates_valid_agent() {
        let agent = build_plist(
            "entropy-gc",
            "/usr/local/bin/reck",
            "my-app",
            "entropy-gc.dot",
            "0 3 * * *",
            Path::new("/tmp/logs"),
        )
        .unwrap();

        assert_eq!(agent.label, "com.reckoner.entropy-gc");
        assert_eq!(agent.process_type, "Background");
        assert_eq!(agent.low_priority_io, Some(true));
        assert!(agent.program_arguments.contains(&"task".to_string()));
        assert!(agent.program_arguments.contains(&"my-app".to_string()));
        assert!(agent.program_arguments.contains(&"--no-pr".to_string()));
        assert_eq!(agent.start_calendar_interval.hour, Some(3));
        assert_eq!(agent.start_calendar_interval.minute, Some(0));
        assert!(agent.standard_out_path.contains("entropy-gc.stdout.log"));
        assert!(agent.environment_variables.contains_key("PATH"));
        assert!(agent.environment_variables["PATH"].contains("/opt/homebrew/bin"));
    }

    #[test]
    fn build_plist_with_weekly_schedule() {
        let agent = build_plist(
            "weekly-scan",
            "/usr/local/bin/reck",
            "repo",
            "scan.dot",
            "30 2 * * 0",
            Path::new("/tmp/logs"),
        )
        .unwrap();

        assert_eq!(agent.start_calendar_interval.hour, Some(2));
        assert_eq!(agent.start_calendar_interval.minute, Some(30));
        assert_eq!(agent.start_calendar_interval.weekday, Some(0));
    }

    #[test]
    fn build_plist_rejects_bad_cron() {
        let result = build_plist(
            "bad",
            "/usr/local/bin/reck",
            "repo",
            "p.dot",
            "invalid",
            Path::new("/tmp"),
        );
        assert!(result.is_err());
    }

    // ── plist serialization ──────────────────────────────────────────

    #[test]
    fn plist_serializes_to_xml() {
        let agent = build_plist(
            "test",
            "/usr/local/bin/reck",
            "repo",
            "test.dot",
            "0 3 * * *",
            Path::new("/tmp/logs"),
        )
        .unwrap();

        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &agent).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("com.reckoner.test"));
        assert!(xml.contains("/usr/local/bin/reck"));
        assert!(xml.contains("Background"));
        assert!(xml.contains("<key>Hour</key>"));
    }

    #[test]
    fn plist_write_to_file_and_read_back() {
        let dir = TempDir::new().unwrap();
        let agent = build_plist(
            "roundtrip",
            "/usr/local/bin/reck",
            "repo",
            "test.dot",
            "15 4 * * *",
            Path::new("/tmp/logs"),
        )
        .unwrap();

        let path = dir.path().join("test.plist");
        plist::to_file_xml(&path, &agent).unwrap();

        let read_back: LaunchAgent = plist::from_file(&path).unwrap();
        assert_eq!(read_back.label, "com.reckoner.roundtrip");
        assert_eq!(read_back.start_calendar_interval.hour, Some(4));
        assert_eq!(read_back.start_calendar_interval.minute, Some(15));
    }

    // ── CalendarInterval serialization ───────────────────────────────

    #[test]
    fn calendar_interval_omits_none_fields() {
        let ci = CalendarInterval {
            hour: Some(3),
            minute: Some(0),
            ..Default::default()
        };

        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &ci).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("<key>Hour</key>"));
        assert!(xml.contains("<key>Minute</key>"));
        assert!(!xml.contains("<key>Weekday</key>"));
        assert!(!xml.contains("<key>Day</key>"));
        assert!(!xml.contains("<key>Month</key>"));
    }

    // ── list_schedules ───────────────────────────────────────────────
    // (Can't easily test without mocking HOME, but verify the function signature compiles)

    #[test]
    fn list_schedules_returns_vec() {
        // This just verifies the function compiles and returns the right type.
        // Actual LaunchAgents dir may or may not have reckoner plists.
        let result = list_schedules();
        assert!(result.is_ok());
    }
}
