use chrono::Utc;
use rusqlite::{params, Connection, Result};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueStats {
    pub total: u64,
    pub unsynced: u64,
    pub synced: u64,
    pub last_attempt: Option<String>,
}

pub struct QueueDb {
    path: String,
}

impl QueueDb {
    pub fn new(path: &str) -> Result<Self> {
        let db = Self {
            path: path.to_string(),
        };
        db.init()?;
        Ok(db)
    }

    fn connect(&self) -> Result<Connection> {
        if let Some(parent) = Path::new(&self.path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        Connection::open(&self.path)
    }

    pub fn init(&self) -> Result<()> {
        let conn = self.connect()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS queue_payloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                synced INTEGER DEFAULT 0,
                retry_count INTEGER DEFAULT 0,
                last_attempt TEXT,
                last_error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_queue_synced ON queue_payloads(synced);
            "#,
        )?;
        Ok(())
    }

    pub fn enqueue(&self, payload: &str) -> Result<i64> {
        let conn = self.connect()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO queue_payloads (payload, created_at, synced) VALUES (?1, ?2, 0)",
            params![payload, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn mark_synced(&self, id: i64) -> Result<()> {
        let conn = self.connect()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE queue_payloads SET synced = 1, last_attempt = ?1, last_error = NULL WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: i64, error_msg: &str) -> Result<()> {
        let conn = self.connect()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE queue_payloads SET retry_count = retry_count + 1, last_attempt = ?1, last_error = ?2 WHERE id = ?3",
            params![now, error_msg, id],
        )?;
        Ok(())
    }

    pub fn get_unsynced(&self) -> Result<Vec<(i64, String)>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT id, payload FROM queue_payloads WHERE synced = 0 ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((id, payload))
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    pub fn get_stats(&self) -> Result<QueueStats> {
        let conn = self.connect()?;
        let total: u64 = conn.query_row(
            "SELECT COUNT(*) FROM queue_payloads",
            [],
            |r| {
                let v: i64 = r.get(0)?;
                Ok(v.max(0) as u64)
            },
        ).unwrap_or(0);

        let unsynced: u64 = conn.query_row(
            "SELECT COUNT(*) FROM queue_payloads WHERE synced = 0",
            [],
            |r| {
                let v: i64 = r.get(0)?;
                Ok(v.max(0) as u64)
            },
        ).unwrap_or(0);

        let synced: u64 = conn.query_row(
            "SELECT COUNT(*) FROM queue_payloads WHERE synced = 1",
            [],
            |r| {
                let v: i64 = r.get(0)?;
                Ok(v.max(0) as u64)
            },
        ).unwrap_or(0);

        let last_attempt: Option<String> = conn.query_row(
            "SELECT last_attempt FROM queue_payloads WHERE last_attempt IS NOT NULL ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        ).ok();

        Ok(QueueStats {
            total,
            unsynced,
            synced,
            last_attempt,
        })
    }
}

// -------------------------------------------------------------------------
// Reporting SQLite Database Reader & Mock Seeder
// -------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReportingData {
    pub usage_count: u64,
    pub software_active_hours: f64,
    pub light_source_hours: f64,
    pub total_patients: u64,
    pub total_doctors: u64,
    pub total_examinations: u64,
    pub total_reports_generated: u64,
    pub detected_errors: Vec<RawDetectedError>,
}

#[derive(Debug, Clone)]
pub struct RawDetectedError {
    pub timestamp: String,
    pub module: String,
    pub error_message: String,
}

pub struct ReportingDbReader;

impl ReportingDbReader {
    pub fn read(db_path: &str) -> Result<ReportingData, Box<dyn std::error::Error>> {
        let conn = Connection::open(db_path)?;

        // 1. Read usage metrics
        let (usage_count, software_active_hours, light_source_hours) = Self::read_usage_metrics(&conn)?;

        // 2. Read productivity metrics
        let (total_patients, total_doctors, total_examinations, total_reports_generated) =
            Self::read_productivity_metrics(&conn)?;

        // 3. Read detected errors
        let detected_errors = Self::read_and_flag_errors(&conn)?;

        Ok(ReportingData {
            usage_count,
            software_active_hours,
            light_source_hours,
            total_patients,
            total_doctors,
            total_examinations,
            total_reports_generated,
            detected_errors,
        })
    }

    fn read_usage_metrics(conn: &Connection) -> Result<(u64, f64, f64)> {
        // Check for dedicated telemetry_metrics table
        if Self::table_exists(conn, "telemetry_metrics")? {
            let row = conn.query_row(
                "SELECT usage_count, software_active_hours, light_source_hours FROM telemetry_metrics LIMIT 1",
                [],
                |r| {
                    let uc: i64 = r.get(0)?;
                    let sh: f64 = r.get(1)?;
                    let lh: f64 = r.get(2)?;
                    Ok((uc.max(0) as u64, sh, lh))
                },
            );
            if let Ok(res) = row {
                return Ok(res);
            }
        }

        // Check for app_usage or launch sessions
        if Self::table_exists(conn, "app_usage")? {
            let row = conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(active_seconds), 0) / 3600.0, COALESCE(SUM(light_seconds), 0) / 3600.0 FROM app_usage",
                [],
                |r| {
                    let count: i64 = r.get(0)?;
                    let sh: f64 = r.get(1)?;
                    let lh: f64 = r.get(2)?;
                    Ok((count.max(0) as u64, sh, lh))
                },
            );
            if let Ok(res) = row {
                return Ok(res);
            }
        }

        // Fallback default
        Ok((0, 0.0, 0.0))
    }

    fn read_productivity_metrics(conn: &Connection) -> Result<(u64, u64, u64, u64)> {
        // Direct telemetry_productivity table if exists
        if Self::table_exists(conn, "telemetry_productivity")? {
            let row = conn.query_row(
                "SELECT total_patients, total_doctors, total_examinations, total_reports_generated FROM telemetry_productivity LIMIT 1",
                [],
                |r| {
                    let p: i64 = r.get(0)?;
                    let d: i64 = r.get(1)?;
                    let e: i64 = r.get(2)?;
                    let rep: i64 = r.get(3)?;
                    Ok((p.max(0) as u64, d.max(0) as u64, e.max(0) as u64, rep.max(0) as u64))
                },
            );
            if let Ok(res) = row {
                return Ok(res);
            }
        }

        // Otherwise query individual clinical tables if present
        let total_patients = Self::count_table_rows(conn, "patients").unwrap_or(0);
        let total_doctors = Self::count_table_rows(conn, "doctors").unwrap_or(0);
        let total_examinations = Self::count_table_rows(conn, "examinations").unwrap_or(0);
        let total_reports_generated = Self::count_table_rows(conn, "reports").unwrap_or(0);

        Ok((total_patients, total_doctors, total_examinations, total_reports_generated))
    }

    fn read_and_flag_errors(conn: &Connection) -> Result<Vec<RawDetectedError>> {
        let mut errors = Vec::new();

        let table_name = if Self::table_exists(conn, "telemetry_errors")? {
            "telemetry_errors"
        } else if Self::table_exists(conn, "error_logs")? {
            "error_logs"
        } else {
            return Ok(errors);
        };

        let has_reported_col = conn
            .prepare(&format!("PRAGMA table_info({})", table_name))?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|col| col == "reported");

        let query = if has_reported_col {
            format!(
                "SELECT id, timestamp, module, error_message FROM {} WHERE reported = 0 ORDER BY id ASC LIMIT 20",
                table_name
            )
        } else {
            format!(
                "SELECT id, timestamp, module, error_message FROM {} ORDER BY id DESC LIMIT 10",
                table_name
            )
        };

        let mut ids_to_mark = Vec::new();
        {
            let mut stmt = conn.prepare(&query)?;
            let rows = stmt.query_map([], |row| {
                let id: i64 = row.get(0)?;
                let timestamp: String = row.get(1)?;
                let module: String = row.get(2)?;
                let error_message: String = row.get(3)?;
                Ok((id, RawDetectedError { timestamp, module, error_message }))
            })?;

            for r in rows {
                if let Ok((id, err)) = r {
                    ids_to_mark.push(id);
                    errors.push(err);
                }
            }
        }

        if has_reported_col && !ids_to_mark.is_empty() {
            for id in ids_to_mark {
                let _ = conn.execute(
                    &format!("UPDATE {} SET reported = 1 WHERE id = ?1", table_name),
                    params![id],
                );
            }
        }

        Ok(errors)
    }

    fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    fn count_table_rows(conn: &Connection, table_name: &str) -> Result<u64> {
        if !Self::table_exists(conn, table_name)? {
            return Ok(0);
        }
        let count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {}", table_name),
            [],
            |r| r.get(0),
        )?;
        Ok(count.max(0) as u64)
    }
}

/// Helper to create and seed a sample reporting database for development and testing
pub fn seed_sample_reporting_db(db_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = Path::new(db_path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS telemetry_metrics (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            usage_count INTEGER DEFAULT 0,
            software_active_hours REAL DEFAULT 0.0,
            light_source_hours REAL DEFAULT 0.0
        );

        CREATE TABLE IF NOT EXISTS telemetry_productivity (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            total_patients INTEGER DEFAULT 0,
            total_doctors INTEGER DEFAULT 0,
            total_examinations INTEGER DEFAULT 0,
            total_reports_generated INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS telemetry_errors (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            module TEXT NOT NULL,
            error_message TEXT NOT NULL,
            reported INTEGER DEFAULT 0
        );

        INSERT OR REPLACE INTO telemetry_metrics (id, usage_count, software_active_hours, light_source_hours)
        VALUES (1, 142, 328.5, 245.0);

        INSERT OR REPLACE INTO telemetry_productivity (id, total_patients, total_doctors, total_examinations, total_reports_generated)
        VALUES (1, 420, 6, 890, 885);

        INSERT INTO telemetry_errors (timestamp, module, error_message, reported)
        VALUES 
            ('2026-10-27T08:14:22Z', 'video-stream', 'Capture card DirectShow timeout on frame grabber', 0),
            ('2026-10-27T09:30:10Z', 'fo-dicom', 'PACS archive connection refused: 192.168.1.50:104', 0);
        "#,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_db_operations() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_queue_{}.db", Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let path_str = db_file.to_str().unwrap();

        let queue = QueueDb::new(path_str).expect("Failed to create queue db");
        
        let stats = queue.get_stats().expect("stats");
        assert_eq!(stats.total, 0);
        assert_eq!(stats.unsynced, 0);

        let id1 = queue.enqueue(r#"{"test": 1}"#).expect("enqueue 1");
        let id2 = queue.enqueue(r#"{"test": 2}"#).expect("enqueue 2");

        let unsynced = queue.get_unsynced().expect("get unsynced");
        assert_eq!(unsynced.len(), 2);
        assert_eq!(unsynced[0].0, id1);
        assert_eq!(unsynced[1].0, id2);

        queue.mark_synced(id1).expect("mark synced");
        let unsynced_after = queue.get_unsynced().expect("get unsynced after");
        assert_eq!(unsynced_after.len(), 1);
        assert_eq!(unsynced_after[0].0, id2);

        let stats_after = queue.get_stats().expect("stats after");
        assert_eq!(stats_after.total, 2);
        assert_eq!(stats_after.synced, 1);
        assert_eq!(stats_after.unsynced, 1);

        let _ = std::fs::remove_file(db_file);
    }

    #[test]
    fn test_reporting_db_seed_and_read() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join(format!("test_rep_{}.db", Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        let path_str = db_file.to_str().unwrap();

        seed_sample_reporting_db(path_str).expect("Seed should succeed");
        let data = ReportingDbReader::read(path_str).expect("Read should succeed");

        assert_eq!(data.usage_count, 142);
        assert_eq!(data.software_active_hours, 328.5);
        assert_eq!(data.light_source_hours, 245.0);
        assert_eq!(data.total_patients, 420);
        assert_eq!(data.total_doctors, 6);
        assert_eq!(data.total_examinations, 890);
        assert_eq!(data.total_reports_generated, 885);
        assert_eq!(data.detected_errors.len(), 2);

        let _ = std::fs::remove_file(db_file);
    }
}
