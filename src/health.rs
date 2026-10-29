use crate::collector::DetectedError;
use crate::config::Config;
use crate::db::QueueDb;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use sysinfo::{Pid, System};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentHealthReport {
    pub agent_uptime_seconds: f64,
    pub memory_rss_mb: f64,
    pub cpu_pct: f64,
    pub last_sync_at: Option<String>,
    pub queue_depth: u64,
    pub db_size_mb: f64,
    pub error_count_24h: u64,
    pub version: String,
    pub is_connected_to_server: bool,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
    Offline,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "Healthy"),
            HealthStatus::Warning => write!(f, "Warning"),
            HealthStatus::Critical => write!(f, "Critical"),
            HealthStatus::Offline => write!(f, "Offline"),
        }
    }
}

/// Collect a live health snapshot for the running agent.
pub async fn collect_health_report(
    config: &Config,
    queue_db: &QueueDb,
    detected_errors: &[DetectedError],
) -> Result<AgentHealthReport, Box<dyn std::error::Error>> {
    let (uptime_seconds, memory_rss_mb, cpu_pct) = collect_system_metrics();

    let stats = queue_db.get_stats()?;
    let queue_depth = stats.unsynced;
    let last_sync_at = stats.last_attempt.clone();

    let db_size_mb = if Path::new(&config.db_path).exists() {
        let meta = fs::metadata(&config.db_path)?;
        meta.len() as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    };

    let now = Utc::now();
    let cutoff = now - Duration::hours(24);
    let error_count_24h = detected_errors
        .iter()
        .filter(|e| parse_ts_ge(&e.timestamp, &cutoff))
        .count() as u64;

    let last_error_message = detected_errors.last().map(|e| e.error_message.clone());

    let is_connected_to_server = ping_server(config).await;

    Ok(AgentHealthReport {
        agent_uptime_seconds: uptime_seconds,
        memory_rss_mb,
        cpu_pct,
        last_sync_at,
        queue_depth,
        db_size_mb,
        error_count_24h,
        version: config.software_version.clone(),
        is_connected_to_server,
        last_error_message,
    })
}

pub fn calculate_status(report: &AgentHealthReport) -> HealthStatus {
    let depth = report.queue_depth;
    let errors = report.error_count_24h;
    let connected = report.is_connected_to_server;
    let uptime = report.agent_uptime_seconds;

    if depth > 50 || errors > 5 || !connected {
        return HealthStatus::Critical;
    }
    if depth >= 10 || errors > 0 || uptime < 3600.0 {
        return HealthStatus::Warning;
    }
    HealthStatus::Healthy
}

async fn ping_server(config: &Config) -> bool {
    let client = match crate::client::TelemetryClient::new(&config.server_url) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let url = client.server_url().to_string();
    match reqwest::get(&url).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

fn collect_system_metrics() -> (f64, f64, f64) {
    let mut sys = System::new_all();
    sys.refresh_all();
    let pid = Pid::from(std::process::id() as usize);
    let memory_rss_mb = sys
        .process(pid)
        .map(|p| p.memory() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);
    let uptime_seconds = System::uptime() as f64;
    let cpu_pct = sys
        .process(pid)
        .map(|p| p.cpu_usage() as f64)
        .unwrap_or(sys.global_cpu_usage() as f64);
    (uptime_seconds, memory_rss_mb, cpu_pct)
}

fn parse_ts_ge(timestamp: &str, cutoff: &chrono::DateTime<chrono::Utc>) -> bool {
    match chrono::DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => dt >= *cutoff,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_baseline() {
        let report = AgentHealthReport {
            agent_uptime_seconds: 7200.0,
            memory_rss_mb: 12.0,
            cpu_pct: 4.5,
            last_sync_at: Some("2026-10-29T10:00:00Z".to_string()),
            queue_depth: 3,
            db_size_mb: 0.5,
            error_count_24h: 0,
            version: "1.2.0".to_string(),
            is_connected_to_server: true,
            last_error_message: None,
        };
        assert_eq!(calculate_status(&report), HealthStatus::Healthy);
    }

    #[test]
    fn warning_queue_depth_medium() {
        let mut report = base_report();
        report.queue_depth = 25;
        assert_eq!(calculate_status(&report), HealthStatus::Warning);
    }

    #[test]
    fn warning_some_errors() {
        let mut report = base_report();
        report.error_count_24h = 2;
        assert_eq!(calculate_status(&report), HealthStatus::Warning);
    }

    #[test]
    fn warning_low_uptime() {
        let mut report = base_report();
        report.agent_uptime_seconds = 1800.0;
        assert_eq!(calculate_status(&report), HealthStatus::Warning);
    }

    #[test]
    fn critical_queue_depth_high() {
        let mut report = base_report();
        report.queue_depth = 120;
        assert_eq!(calculate_status(&report), HealthStatus::Critical);
    }

    #[test]
    fn critical_many_errors() {
        let mut report = base_report();
        report.error_count_24h = 8;
        assert_eq!(calculate_status(&report), HealthStatus::Critical);
    }

    #[test]
    fn critical_not_connected() {
        let mut report = base_report();
        report.is_connected_to_server = false;
        assert_eq!(calculate_status(&report), HealthStatus::Critical);
    }

    #[test]
    fn offline_status_not_used_yet_but_present() {
        let report = base_report();
        // Offline is reserved for fatal errors / process state; current thresholds map to Warning/Critical.
        assert_ne!(calculate_status(&report), HealthStatus::Offline);
    }

    fn base_report() -> AgentHealthReport {
        AgentHealthReport {
            agent_uptime_seconds: 7200.0,
            memory_rss_mb: 10.0,
            cpu_pct: 2.0,
            last_sync_at: None,
            queue_depth: 0,
            db_size_mb: 0.0,
            error_count_24h: 0,
            version: "1.2.0".to_string(),
            is_connected_to_server: true,
            last_error_message: None,
        }
    }
}
