use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SoftwareName {
    Glescope,
    Antech,
    Easttech,
    Zeus,
}

impl Default for SoftwareName {
    fn default() -> Self {
        SoftwareName::Glescope
    }
}

impl fmt::Display for SoftwareName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SoftwareName::Glescope => write!(f, "GLESCOPE"),
            SoftwareName::Antech => write!(f, "ANTECH"),
            SoftwareName::Easttech => write!(f, "EASTTECH"),
            SoftwareName::Zeus => write!(f, "ZEUS"),
        }
    }
}

impl FromStr for SoftwareName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "GLESCOPE" => Ok(SoftwareName::Glescope),
            "ANTECH" => Ok(SoftwareName::Antech),
            "EASTTECH" => Ok(SoftwareName::Easttech),
            "ZEUS" => Ok(SoftwareName::Zeus),
            other => Err(format!(
                "Invalid software name: '{}'. Must be one of GLESCOPE, ANTECH, EASTTECH, ZEUS",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server_url: String,
    pub device_id: Option<String>,
    pub software_name: SoftwareName,
    pub software_version: String,
    pub hospital_name: String,
    pub region: String,
    pub city: String,
    pub room_name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub db_path: String,
    pub queue_db_path: String,
    pub interval_secs: u64,
    pub enable_mock_if_missing: bool,
}

impl Default for Config {
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        let default_db_path = r"D:\GLESCOPE_DB\telemetry.db".to_string();
        #[cfg(not(target_os = "windows"))]
        let default_db_path = "./data/telemetry.db".to_string();

        Self {
            server_url: "https://apig.fathforce.com/v1/update/check".to_string(),
            device_id: None,
            software_name: SoftwareName::Glescope,
            software_version: "1.2.0".to_string(),
            hospital_name: "RSUD Dr. Soetomo".to_string(),
            region: "Jawa Timur".to_string(),
            city: "Surabaya".to_string(),
            room_name: "Poli THT - Ruang Tindakan 2".to_string(),
            latitude: -7.2575,
            longitude: 112.7521,
            db_path: default_db_path,
            queue_db_path: "./agent_queue.db".to_string(),
            interval_secs: 300,
            enable_mock_if_missing: true,
        }
    }
}

impl Config {
    /// Load config from a JSON or TOML file if present, then apply env overrides
    pub fn load_or_default(config_path: Option<&str>) -> Self {
        let mut cfg = if let Some(path_str) = config_path {
            let path = Path::new(path_str);
            if path.exists() {
                Self::from_file(path).unwrap_or_else(|e| {
                    eprintln!("Warning: Failed to parse config at {:?}: {}. Using defaults.", path, e);
                    Self::default()
                })
            } else {
                Self::default()
            }
        } else if Path::new("agent_config.json").exists() {
            Self::from_file(Path::new("agent_config.json")).unwrap_or_default()
        } else {
            Self::default()
        };

        cfg.apply_env_overrides();
        cfg
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let cfg: Config = serde_json::from_str(&content)?;
        Ok(cfg)
    }

    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("GTEL_SERVER_URL") {
            if !v.trim().is_empty() {
                self.server_url = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_DEVICE_ID") {
            if !v.trim().is_empty() {
                self.device_id = Some(v.trim().to_string());
            }
        }
        if let Ok(v) = std::env::var("GTEL_SOFTWARE_NAME") {
            if let Ok(sn) = SoftwareName::from_str(&v) {
                self.software_name = sn;
            }
        }
        if let Ok(v) = std::env::var("GTEL_SOFTWARE_VERSION") {
            if !v.trim().is_empty() {
                self.software_version = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_HOSPITAL_NAME") {
            if !v.trim().is_empty() {
                self.hospital_name = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_REGION") {
            if !v.trim().is_empty() {
                self.region = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_CITY") {
            if !v.trim().is_empty() {
                self.city = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_ROOM_NAME") {
            if !v.trim().is_empty() {
                self.room_name = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_LATITUDE") {
            if let Ok(lat) = v.parse::<f64>() {
                self.latitude = lat;
            }
        }
        if let Ok(v) = std::env::var("GTEL_LONGITUDE") {
            if let Ok(lon) = v.parse::<f64>() {
                self.longitude = lon;
            }
        }
        if let Ok(v) = std::env::var("GTEL_DB_PATH") {
            if !v.trim().is_empty() {
                self.db_path = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_QUEUE_DB_PATH") {
            if !v.trim().is_empty() {
                self.queue_db_path = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GTEL_INTERVAL_SECS") {
            if let Ok(secs) = v.parse::<u64>() {
                self.interval_secs = secs;
            }
        }
        if let Ok(v) = std::env::var("GTEL_ENABLE_MOCK") {
            self.enable_mock_if_missing = v == "1" || v.eq_ignore_ascii_case("true");
        }
    }

    /// Resolve effective device ID (configured override or HWID)
    pub fn get_effective_device_id(&self, hwid: &str) -> String {
        if let Some(ref id) = self.device_id {
            if !id.trim().is_empty() {
                return id.trim().to_string();
            }
        }
        hwid.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_software_name_serialization() {
        assert_eq!(serde_json::to_string(&SoftwareName::Glescope).unwrap(), "\"GLESCOPE\"");
        assert_eq!(serde_json::to_string(&SoftwareName::Antech).unwrap(), "\"ANTECH\"");
        assert_eq!(serde_json::to_string(&SoftwareName::Easttech).unwrap(), "\"EASTTECH\"");
        assert_eq!(serde_json::to_string(&SoftwareName::Zeus).unwrap(), "\"ZEUS\"");

        let s: SoftwareName = serde_json::from_str("\"GLESCOPE\"").unwrap();
        assert_eq!(s, SoftwareName::Glescope);
    }

    #[test]
    fn test_config_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.server_url, "https://apig.fathforce.com/v1/update/check");
        assert_eq!(cfg.software_name, SoftwareName::Glescope);
        assert_eq!(cfg.software_version, "1.2.0");
        assert_eq!(cfg.hospital_name, "RSUD Dr. Soetomo");
        assert_eq!(cfg.get_effective_device_id("DEV-MOCK123"), "DEV-MOCK123");
    }

    #[test]
    fn test_config_device_id_override() {
        let mut cfg = Config::default();
        cfg.device_id = Some("DEV-CUSTOM-001".to_string());
        assert_eq!(cfg.get_effective_device_id("DEV-MOCK123"), "DEV-CUSTOM-001");
    }
}
