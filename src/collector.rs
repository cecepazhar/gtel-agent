use crate::config::Config;
use crate::db::ReportingDbReader;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HospitalInfo {
    pub hospital_name: String,
    pub region: String,
    pub city: String,
    pub room_name: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageMetrics {
    pub usage_count: u64,
    pub software_active_hours: f64,
    pub light_source_hours: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProductivityMetrics {
    pub total_patients: u64,
    pub total_doctors: u64,
    pub total_examinations: u64,
    pub total_reports_generated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedError {
    pub timestamp: String,
    pub module: String,
    pub error_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelemetryPayload {
    pub device_id: String,
    pub software_name: String,
    pub software_version: String,
    pub hospital_info: HospitalInfo,
    pub usage_metrics: UsageMetrics,
    pub productivity_metrics: ProductivityMetrics,
    pub detected_errors: Vec<DetectedError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_report: Option<crate::health::AgentHealthReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerResponse {
    pub status: String,
    pub update_available: bool,
    pub latest_version: String,
    #[serde(default)]
    pub server_timestamp: Option<String>,
}

/// Collects telemetry snapshot from reporting database or mock fallback
pub fn collect_telemetry(config: &Config, hwid: &str) -> Result<TelemetryPayload, Box<dyn std::error::Error>> {
    let device_id = config.get_effective_device_id(hwid);
    let db_path = &config.db_path;

    let (usage_metrics, productivity_metrics, detected_errors) = if Path::new(db_path).exists() {
        let rep_data = ReportingDbReader::read(db_path)?;
        let errors = rep_data
            .detected_errors
            .into_iter()
            .map(|e| DetectedError {
                timestamp: e.timestamp,
                module: e.module,
                error_message: e.error_message,
            })
            .collect();

        (
            UsageMetrics {
                usage_count: rep_data.usage_count,
                software_active_hours: rep_data.software_active_hours,
                light_source_hours: rep_data.light_source_hours,
            },
            ProductivityMetrics {
                total_patients: rep_data.total_patients,
                total_doctors: rep_data.total_doctors,
                total_examinations: rep_data.total_examinations,
                total_reports_generated: rep_data.total_reports_generated,
            },
            errors,
        )
    } else if config.enable_mock_if_missing {
        // Mock fallback for development or non-reporting PCs
        generate_mock_metrics()
    } else {
        return Err(format!(
            "Reporting SQLite database not found at '{}' and mock generator is disabled",
            db_path
        )
        .into());
    };

    Ok(TelemetryPayload {
        device_id,
        software_name: config.software_name.to_string(),
        software_version: config.software_version.clone(),
        hospital_info: HospitalInfo {
            hospital_name: config.hospital_name.clone(),
            region: config.region.clone(),
            city: config.city.clone(),
            room_name: config.room_name.clone(),
            latitude: config.latitude,
            longitude: config.longitude,
        },
        usage_metrics,
        productivity_metrics,
        detected_errors,
        health_report: None,
    })
}

pub fn generate_mock_metrics() -> (UsageMetrics, ProductivityMetrics, Vec<DetectedError>) {
    let now = Utc::now().to_rfc3339();
    let usage = UsageMetrics {
        usage_count: 142,
        software_active_hours: 328.5,
        light_source_hours: 245.0,
    };
    let productivity = ProductivityMetrics {
        total_patients: 420,
        total_doctors: 6,
        total_examinations: 890,
        total_reports_generated: 885,
    };
    let errors = vec![
        DetectedError {
            timestamp: now.clone(),
            module: "video-stream".to_string(),
            error_message: "Simulated DirectShow frame grabber timeout warning".to_string(),
        },
        DetectedError {
            timestamp: now,
            module: "fo-dicom".to_string(),
            error_message: "Simulated PACS archive connection retry (mock)".to_string(),
        },
    ];

    (usage, productivity, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_payload_json_structure() {
        let sample_json = r#"{
  "device_id": "DEV-GLS-SUB-001",
  "software_name": "GLESCOPE",
  "software_version": "1.2.0",
  "hospital_info": {
    "hospital_name": "RSUD Dr. Soetomo",
    "region": "Jawa Timur",
    "city": "Surabaya",
    "room_name": "Poli THT - Ruang Tindakan 2",
    "latitude": -7.2575,
    "longitude": 112.7521
  },
  "usage_metrics": {
    "usage_count": 142,
    "software_active_hours": 328.5,
    "light_source_hours": 245.0
  },
  "productivity_metrics": {
    "total_patients": 420,
    "total_doctors": 6,
    "total_examinations": 890,
    "total_reports_generated": 885
  },
  "detected_errors": [
    {
      "timestamp": "2026-10-27T08:14:22Z",
      "module": "video-stream",
      "error_message": "Capture card DirectShow timeout on frame grabber"
    },
    {
      "timestamp": "2026-10-27T09:30:10Z",
      "module": "fo-dicom",
      "error_message": "PACS archive connection refused: 192.168.1.50:104"
    }
  ]
}"#;

        let parsed: TelemetryPayload = serde_json::from_str(sample_json).expect("Deserialization failed");
        assert_eq!(parsed.device_id, "DEV-GLS-SUB-001");
        assert_eq!(parsed.software_name, "GLESCOPE");
        assert_eq!(parsed.usage_metrics.usage_count, 142);
        assert_eq!(parsed.productivity_metrics.total_patients, 420);
        assert_eq!(parsed.detected_errors.len(), 2);
        assert_eq!(parsed.detected_errors[0].module, "video-stream");

        let serialized = serde_json::to_string(&parsed).expect("Serialization failed");
        let re_parsed: TelemetryPayload = serde_json::from_str(&serialized).expect("Re-parse failed");
        assert_eq!(parsed, re_parsed);
    }

    #[test]
    fn test_server_response_deserialization() {
        let resp_json = r#"{
  "status": "success",
  "update_available": false,
  "latest_version": "1.2.0",
  "server_timestamp": "2026-10-28T03:00:00Z"
}"#;

        let resp: ServerResponse = serde_json::from_str(resp_json).expect("Parse server response");
        assert_eq!(resp.status, "success");
        assert!(!resp.update_available);
        assert_eq!(resp.latest_version, "1.2.0");
        assert_eq!(resp.server_timestamp, Some("2026-10-28T03:00:00Z".to_string()));
    }
}
