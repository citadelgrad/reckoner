use std::path::{Path, PathBuf};
use std::process::Command;

/// Check if a binary is available on PATH.
pub fn has_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if `hl` (JSON log viewer) is installed.
pub fn has_hl() -> bool {
    has_binary("hl")
}

/// View a JSONL log file using `hl` if available, otherwise cat.
pub fn view_log_with_hl(path: &Path, filter: Option<&str>) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("log file not found: {}", path.display());
    }

    if has_hl() {
        let mut args = vec![path.to_string_lossy().into_owned()];
        if let Some(f) = filter {
            args.push("--filter".into());
            args.push(f.into());
        }
        let status = Command::new("hl").args(&args).status()?;
        if !status.success() {
            anyhow::bail!("hl exited with code {:?}", status.code());
        }
    } else {
        // Fallback: print lines, optionally filtered
        let content = std::fs::read_to_string(path)?;
        for line in content.lines() {
            if let Some(f) = filter {
                if line.contains(f) {
                    println!("{}", line);
                }
            } else {
                println!("{}", line);
            }
        }
    }

    Ok(())
}

/// Docker compose file content for the observability stack.
pub fn compose_template(loki_port: u16, grafana_port: u16) -> String {
    format!(
        r#"services:
  loki:
    image: grafana/loki:3.4
    ports:
      - "{loki_port}:3100"
    volumes:
      - reckoner-loki-data:/loki
    command: -config.file=/etc/loki/local-config.yaml
    restart: unless-stopped
    healthcheck:
      test: ["CMD-SHELL", "wget -qO- http://localhost:3100/ready || exit 1"]
      interval: 30s
      timeout: 5s
      retries: 3

  grafana:
    image: grafana/grafana:11.5
    ports:
      - "{grafana_port}:3000"
    volumes:
      - reckoner-grafana-data:/var/lib/grafana
    environment:
      GF_AUTH_ANONYMOUS_ENABLED: "true"
      GF_AUTH_ANONYMOUS_ORG_ROLE: Admin
    depends_on:
      loki:
        condition: service_healthy
    restart: unless-stopped

volumes:
  reckoner-loki-data:
    name: reckoner_loki_data
  reckoner-grafana-data:
    name: reckoner_grafana_data
"#
    )
}

/// Path to the infra directory where compose files live.
pub fn infra_dir() -> PathBuf {
    dirs::home_dir()
        .expect("HOME not set")
        .join(".reckoner/infra")
}

/// Write the compose file to the infra directory.
pub fn ensure_compose_file(loki_port: u16, grafana_port: u16) -> anyhow::Result<PathBuf> {
    let dir = infra_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("docker-compose.yml");
    let content = compose_template(loki_port, grafana_port);
    std::fs::write(&path, &content)?;
    Ok(path)
}

/// Run `docker compose up -d` in the infra directory.
pub fn infra_up() -> anyhow::Result<()> {
    let compose_path = ensure_compose_file(3147, 3148)?;
    let dir = compose_path.parent().unwrap();
    tracing::info!("starting observability stack");
    let output = Command::new("docker")
        .args(["compose", "up", "-d"])
        .current_dir(dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker compose up failed: {}", stderr.trim());
    }
    println!("Observability stack started");
    println!("  Loki:    http://localhost:3147");
    println!("  Grafana: http://localhost:3148");
    Ok(())
}

/// Run `docker compose down` in the infra directory.
pub fn infra_down() -> anyhow::Result<()> {
    let dir = infra_dir();
    let compose = dir.join("docker-compose.yml");
    if !compose.exists() {
        anyhow::bail!("no infra running (no docker-compose.yml found)");
    }
    tracing::info!("stopping observability stack");
    let output = Command::new("docker")
        .args(["compose", "down"])
        .current_dir(&dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker compose down failed: {}", stderr.trim());
    }
    println!("Observability stack stopped");
    Ok(())
}

/// Check status of the observability stack.
pub fn infra_status() -> anyhow::Result<String> {
    let dir = infra_dir();
    let compose = dir.join("docker-compose.yml");
    if !compose.exists() {
        return Ok("not configured (run `reck infra up` first)".into());
    }

    let output = Command::new("docker")
        .args(["compose", "ps", "--format", "table {{.Name}}\t{{.Status}}\t{{.Ports}}"])
        .current_dir(&dir)
        .output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            Ok("configured but not running".into())
        } else {
            Ok(stdout)
        }
    } else {
        Ok("configured but docker compose failed to query status".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── has_binary ───────────────────────────────────────────────────

    #[test]
    fn has_binary_finds_common_tools() {
        assert!(has_binary("git"));
        assert!(has_binary("echo"));
    }

    #[test]
    fn has_binary_returns_false_for_missing() {
        assert!(!has_binary("totally_nonexistent_binary_xyz123"));
    }

    // ── has_hl ───────────────────────────────────────────────────────

    #[test]
    fn has_hl_returns_bool() {
        // Just verify it doesn't panic — hl may or may not be installed
        let _ = has_hl();
    }

    // ── compose_template ─────────────────────────────────────────────

    #[test]
    fn compose_template_uses_custom_ports() {
        let yaml = compose_template(9100, 9200);
        assert!(yaml.contains("9100:3100"));
        assert!(yaml.contains("9200:3000"));
    }

    #[test]
    fn compose_template_has_required_services() {
        let yaml = compose_template(3147, 3148);
        assert!(yaml.contains("loki:"));
        assert!(yaml.contains("grafana:"));
        assert!(yaml.contains("reckoner-loki-data"));
        assert!(yaml.contains("reckoner-grafana-data"));
        assert!(yaml.contains("unless-stopped"));
        assert!(yaml.contains("healthcheck"));
    }

    #[test]
    fn compose_template_enables_anonymous_auth() {
        let yaml = compose_template(3147, 3148);
        assert!(yaml.contains("GF_AUTH_ANONYMOUS_ENABLED"));
        assert!(yaml.contains("Admin"));
    }

    #[test]
    fn compose_template_grafana_depends_on_loki() {
        let yaml = compose_template(3147, 3148);
        assert!(yaml.contains("depends_on"));
        assert!(yaml.contains("service_healthy"));
    }

    // ── ensure_compose_file ──────────────────────────────────────────

    #[test]
    fn ensure_compose_file_writes_yaml() {
        // This writes to ~/.reckoner/infra/ which is fine for testing
        let path = ensure_compose_file(3147, 3148).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("loki:"));
        assert!(content.contains("3147:3100"));
    }

    // ── view_log_with_hl ─────────────────────────────────────────────

    #[test]
    fn view_log_with_hl_errors_on_missing_file() {
        let result = view_log_with_hl(Path::new("/nonexistent/file.jsonl"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // ── infra_status ─────────────────────────────────────────────────

    #[test]
    fn infra_status_returns_string() {
        let status = infra_status().unwrap();
        // Should return some string regardless of whether infra is running
        assert!(!status.is_empty());
    }
}
