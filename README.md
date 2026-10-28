# Gerlink TelemetryAgent Client (`gtel-agent`)

**Gerlink Telemetry Ecosystem v2.0 Client**  
High-performance, sovereign telemetry and background update-checking agent for Gerlink medical endoscopy and surgical reporting stations (**Glescope, Antech, Easttech, Zeus**).

---

## Features

- **Single Static Binary:** Written in Rust (2021 edition), compiles cleanly for both Linux and Windows (`x86_64-pc-windows-gnu` / `x86_64-pc-windows-msvc`).
- **Hardware Fingerprinting (HWID):**
  - Generates deterministic, unique machine identity hashing:
    $$\text{HWID} = \text{DEV-} + \text{SHA256}(\text{CPU} \parallel \text{Motherboard UUID} \parallel \text{MAC} \parallel \text{Disk})[0..16]$$
  - Linux DMI product UUID / machine-id extraction.
  - Windows registry `MachineGuid` / WMIC extraction.
- **Reporting Database Reader:**
  - Reads clinical non-PII metrics from local reporting SQLite database (`D:\GLESCOPE_DB\telemetry.db` or configured path):
    - Software active hours & light source runtime hours
    - Patient, doctor, examination, and generated report counts
    - Local medical error and exception logs
  - Built-in mock generator for non-production environments and development machines.
- **Cloaked Update-Checker HTTP Dispatcher:**
  - Endpoint: `POST /v1/update/check` (Default: `https://apig.fathforce.com/v1/update/check`)
  - Cloaked User-Agent: `GerlinkUpdateService/1.2`
  - Seamlessly disguises operational and health telemetry as routine software update checks.
- **Offline Resilience:**
  - Local SQLite FIFO queue (`agent_queue.db`) stores telemetry payloads with `synced = 0`.
  - Automatic replay and syncing when network connectivity is restored.

---

## CLI Interface

```bash
# Display hardware fingerprint, target URL, and queue status
telemetry-agent status

# Run daemon scheduler loop (polls and syncs every N seconds)
telemetry-agent run --interval 300

# Execute immediate single probe & dispatch
telemetry-agent send-once

# Verify SQLite reporting DB connectivity (and optionally seed sample records)
telemetry-agent test-db --path ./data/telemetry.db --seed
```

---

## JSON Payload Specification

```json
{
  "device_id": "DEV-F6691AD1DAF0D6AB",
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
    }
  ]
}
```

---

## Environment Variables

| Variable | Description | Default |
|---|---|---|
| `GTEL_SERVER_URL` | Telemetry / update checker endpoint | `https://apig.fathforce.com/v1/update/check` |
| `GTEL_DEVICE_ID` | Explicit device ID override | Auto HWID |
| `GTEL_SOFTWARE_NAME` | Brand (`GLESCOPE`, `ANTECH`, `EASTTECH`, `ZEUS`) | `GLESCOPE` |
| `GTEL_SOFTWARE_VERSION`| Software version string | `1.2.0` |
| `GTEL_HOSPITAL_NAME` | Healthcare facility name | `RSUD Dr. Soetomo` |
| `GTEL_REGION` | Province | `Jawa Timur` |
| `GTEL_CITY` | City / Regency | `Surabaya` |
| `GTEL_ROOM_NAME` | Operating room / clinic station | `Poli THT - Ruang Tindakan 2` |
| `GTEL_DB_PATH` | Path to reporting SQLite database | `D:\GLESCOPE_DB\telemetry.db` (Win) / `./data/telemetry.db` |
| `GTEL_QUEUE_DB_PATH` | Local offline queue SQLite path | `./agent_queue.db` |
| `GTEL_INTERVAL_SECS` | Daemon tick interval (seconds) | `300` |
| `GTEL_ENABLE_MOCK` | Fallback mock if reporting DB missing | `true` |

---

## Verification & Testing

```bash
cargo test
cargo check
cargo run -- status
```

---

## License

Internal & Proprietary — PT Gerlink Utama Mandiri & Fathforce Ecosystem.
