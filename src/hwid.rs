use sha2::{Digest, Sha256};
use sysinfo::{Disks, Networks, System};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HardwareFingerprint {
    pub cpu: String,
    pub motherboard_uuid: String,
    pub mac_address: String,
    pub disk_serial: String,
    pub raw_fingerprint: String,
    pub hwid: String,
}

impl HardwareFingerprint {
    /// Generate complete hardware fingerprint and derived HWID
    pub fn generate() -> Self {
        let cpu = detect_cpu();
        let motherboard_uuid = detect_motherboard_uuid();
        let mac_address = detect_primary_mac();
        let disk_serial = detect_primary_disk();

        let (raw_fingerprint, hwid) = compute_hwid(&cpu, &motherboard_uuid, &mac_address, &disk_serial);

        Self {
            cpu,
            motherboard_uuid,
            mac_address,
            disk_serial,
            raw_fingerprint,
            hwid,
        }
    }
}

/// Computes raw canonical fingerprint string and hashed HWID (DEV-XXXXXXXXXXXXXXXX)
pub fn compute_hwid(cpu: &str, motherboard_uuid: &str, mac: &str, disk: &str) -> (String, String) {
    let raw = format!("CPU:{}|MB:{}|MAC:{}|DISK:{}", cpu.trim(), motherboard_uuid.trim(), mac.trim(), disk.trim());
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    let hex_full: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    let short_hex = if hex_full.len() >= 16 {
        &hex_full[..16]
    } else {
        &hex_full
    };
    let hwid = format!("DEV-{}", short_hex.to_uppercase());
    (raw, hwid)
}

fn detect_cpu() -> String {
    let mut sys = System::new_all();
    sys.refresh_cpu_all();
    let cpus = sys.cpus();
    if let Some(first) = cpus.first() {
        let brand = first.brand().trim();
        let vendor = first.vendor_id().trim();
        let count = cpus.len();
        format!("{brand} [{vendor}] x{count}")
    } else {
        "GENERIC_CPU_x86_64".to_string()
    }
}

fn detect_motherboard_uuid() -> String {
    // 1. Try Linux standard DMI product UUID
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/sys/class/dmi/id/product_uuid") {
            let uuid = content.trim();
            if !uuid.is_empty() && uuid != "None" {
                return uuid.to_string();
            }
        }
        if let Ok(content) = std::fs::read_to_string("/etc/machine-id") {
            let mid = content.trim();
            if !mid.is_empty() {
                return mid.to_string();
            }
        }
        if let Ok(content) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
            let dbus_id = content.trim();
            if !dbus_id.is_empty() {
                return dbus_id.to_string();
            }
        }
    }

    // 2. Try Windows registry or fallback commands
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("reg")
            .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.contains("MachineGuid") {
                        if let Some(guid) = line.split_whitespace().last() {
                            if !guid.is_empty() {
                                return guid.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Fallback: Hostname + OS identifier
    let host = System::host_name().unwrap_or_else(|| "UNKNOWN_HOST".to_string());
    let os = System::long_os_version().unwrap_or_else(|| "UNKNOWN_OS".to_string());
    format!("FALLBACK-{}-{}", host, os)
}

fn detect_primary_mac() -> String {
    let networks = Networks::new_with_refreshed_list();
    let mut preferred_mac: Option<String> = None;
    let mut fallback_mac: Option<String> = None;

    for (name, data) in networks.iter() {
        let mac = data.mac_address().to_string();
        if mac.is_empty() || mac == "00:00:00:00:00:00" {
            continue;
        }

        let name_lower = name.to_lowercase();
        // Skip virtual or tunnel interfaces if possible
        if name_lower.contains("tailscale")
            || name_lower.contains("docker")
            || name_lower.contains("veth")
            || name_lower.contains("br-")
            || name_lower.contains("vmware")
            || name_lower.contains("vbox")
            || name_lower.contains("loopback")
        {
            if fallback_mac.is_none() {
                fallback_mac = Some(mac);
            }
            continue;
        }

        // Physical candidates
        if name_lower.starts_with("en")
            || name_lower.starts_with("eth")
            || name_lower.starts_with("wl")
            || name_lower.contains("ethernet")
            || name_lower.contains("wi-fi")
        {
            preferred_mac = Some(mac);
            break;
        }

        if fallback_mac.is_none() {
            fallback_mac = Some(mac);
        }
    }

    preferred_mac
        .or(fallback_mac)
        .unwrap_or_else(|| "02:00:00:00:00:01".to_string())
}

fn detect_primary_disk() -> String {
    let disks = Disks::new_with_refreshed_list();
    for disk in disks.iter() {
        let mount = disk.mount_point().to_string_lossy();
        if mount == "/" || mount == "C:\\" || mount == "C:" {
            return format!(
                "{}|{}|{}",
                disk.name().to_string_lossy(),
                disk.file_system().to_string_lossy(),
                disk.total_space()
            );
        }
    }

    if let Some(first) = disks.iter().next() {
        format!(
            "{}|{}|{}",
            first.name().to_string_lossy(),
            first.file_system().to_string_lossy(),
            first.total_space()
        )
    } else {
        "DISK_FALLBACK_PRIMARY".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hwid_deterministic() {
        let (raw1, hwid1) = compute_hwid("Intel i7", "UUID-1234", "AA:BB:CC:DD:EE:FF", "NVME01");
        let (raw2, hwid2) = compute_hwid("Intel i7", "UUID-1234", "AA:BB:CC:DD:EE:FF", "NVME01");
        assert_eq!(raw1, raw2);
        assert_eq!(hwid1, hwid2);
        assert!(hwid1.starts_with("DEV-"));
        assert_eq!(hwid1.len(), 4 + 16);
    }

    #[test]
    fn test_compute_hwid_sensitivity() {
        let (_, hwid1) = compute_hwid("Intel i7", "UUID-1234", "AA:BB:CC:DD:EE:FF", "NVME01");
        let (_, hwid2) = compute_hwid("Intel i7", "UUID-1235", "AA:BB:CC:DD:EE:FF", "NVME01");
        assert_ne!(hwid1, hwid2);
    }

    #[test]
    fn test_generate_hardware_fingerprint() {
        let fp = HardwareFingerprint::generate();
        assert!(!fp.cpu.is_empty());
        assert!(!fp.motherboard_uuid.is_empty());
        assert!(!fp.mac_address.is_empty());
        assert!(!fp.disk_serial.is_empty());
        assert!(!fp.raw_fingerprint.is_empty());
        assert!(fp.hwid.starts_with("DEV-"));
    }
}
