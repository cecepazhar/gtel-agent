# Gerlink TelemetryAgent Client — Multi-Brand

**Gerlink Telemetry Ecosystem v2.0 Client (Multi-Brand)**  
High-performance, sovereign telemetry and background update-checking agent for Gerlink medical endoscopy and surgical reporting stations (**Glescope, Antech, Easttech, Zeus**).

> Opsi B arsitektur: 1 binary track semua brand via `profiles` di `agent_config.json`.

---

## Features

- **Multi-Brand, Single Binary:** Satu proses daemon bisa meng-track 4 brand (Glescope, Antech, Easttech, Zeus) sekaligus. Tiap brand punya profil DB + queue terpisah.
- **Hardware Fingerprinting (HWID):**
  - Hash determinan `DEV-` + `SHA256(CPU || Motherboard UUID || MAC || Disk)[0..16]`
  - Windows registry `MachineGuid` / WMIC extraction
  - Linux DMI product UUID / machine-id
- **Reporting Database Reader:**
  - Membaca metrik non-PII dari SQLite report tiap brand (`D:\GLESCOPE_DB\telemetry.db`, `D:\ANTECH_DB\telemetry.db`, dsb)
  - Mock generator untuk PC dev / non-reporting
- **Cloaked Update-Checker Dispatcher:**
  - `POST /v1/update/check` (default `https://apig.fathforce.com/v1/update/check`)
  - User-Agent `GerlinkUpdateService/1.2`
- **Offline Resilience:**
  - Tiap brand punya `agent_queue_<brand>.db` (SQLite FIFO)
  - Replay otomatis saat online

---

## CLI Commands

```bash
# Jalankan daemon multi-brand (interval default dari config / 300 detik)
telemetry-agent run --config agent_config.json

# Eksekusi probe one-shot semua brand
telemetry-agent send-once --config agent_config.json

# Cek status HWID, server, dan 4 profil brand
telemetry-agent status --config agent_config.json

# Cek health tiap brand
telemetry-agent health --config agent_config.json

# Generate config template 4 brand
telemetry-agent init-config --output agent_config.json

# Inspect / seed DB report satu brand
telemetry-agent test-db --path "D:\GLESCOPE_DB\telemetry.db" --seed
```

---

## Struktur `agent_config.json`

```json
{
  "server_url": "https://apig.fathforce.com/v1/update/check",
  "hospital_name": "RSUD Dr. Soetomo",
  "region": "Jawa Timur",
  "city": "Surabaya",
  "room_name": "Poli THT - Ruang Tindakan 2",
  "latitude": -7.2575,
  "longitude": 112.7521,
  "interval_secs": 300,
  "enable_mock_if_missing": true,
  "profiles": [
    {
      "software_name": "GLESCOPE",
      "software_version": "1.2.0",
      "db_path": "D:\\GLESCOPE_DB\\telemetry.db",
      "queue_db_path": "./agent_queue_glescope.db",
      "device_id": null
    },
    {
      "software_name": "ANTECH",
      "software_version": "1.2.0",
      "db_path": "D:\\ANTECH_DB\\telemetry.db",
      "queue_db_path": "./agent_queue_antech.db",
      "device_id": null
    },
    {
      "software_name": "EASTTECH",
      "software_version": "1.2.0",
      "db_path": "D:\\EASTTECH_DB\\telemetry.db",
      "queue_db_path": "./agent_queue_easttech.db",
      "device_id": null
    },
    {
      "software_name": "ZEUS",
      "software_version": "1.2.0",
      "db_path": "D:\\ZEUS_DB\\telemetry.db",
      "queue_db_path": "./agent_queue_zeus.db",
      "device_id": null
    }
  ]
}
```

Aturan:
- `profiles` kosong → fallback ke profil Glescope default (kompatibilitas lama).
- `device_id` null → gunakan HWID lokal; jika diisi, override per brand.
- `server_url`, `hospital_name`, `region`, `city`, `room_name`, `latitude`, `longitude`, `interval_secs`, `enable_mock_if_missing` bersifat **shared** di level atas.

---

## Distribusi .exe Multi-PC

- Build:
  ```bash
  cargo build --release
  ```
- Hasil: `target/release/telemetry-agent.exe` (static binary, tanpa VCRuntime dependensi).
- Isi paket deploy:
  ```
  dist/
  ├── telemetry-agent.exe
  ├── agent_config.json
  └── README.md
  ```
- Install: copy ke `C:\ProgramData\Gerlink\telemetry-agent\` (atau sesuai kebutuhan), lalu daftarkan ke Windows Task Scheduler:
  ```powershell
  schtasks /Create /SC DAILY /TN "Gerlink TelemetryAgent" /TR "C:\ProgramData\Gerlink\telemetry-agent\telemetry-agent.exe run --config C:\ProgramData\Gerlink\telemetry-agent\agent_config.json"
  ```
- Untuk build cross-compile Linux → Windows (GNU) di server zpc:
  ```bash
  rustup target add x86_64-pc-windows-gnu
  cargo build --release --target x86_64-pc-windows-gnu
  ```
  Hasil di `target/x86_64-pc-windows-gnu/release/telemetry-agent.exe`.

---

## Environment Variables (legacy, still valid)

`GTEL_SERVER_URL`, `GTEL_DEVICE_ID`, `GTEL_SOFTWARE_NAME`, `GTEL_SOFTWARE_VERSION`, `GTEL_HOSPITAL_NAME`, `GTEL_REGION`, `GTEL_CITY`, `GTEL_ROOM_NAME`, `GTEL_LATITUDE`, `GTEL_LONGITUDE`, `GTEL_DB_PATH`, `GTEL_QUEUE_DB_PATH`, `GTEL_INTERVAL_SECS`, `GTEL_ENABLE_MOCK`

> `GTEL_DB_PATH` / `GTEL_QUEUE_DB_PATH` sekarang hanya mempengaruhi profil **legacy single-profile** ketika `profiles` kosong. Untuk multi-brand, edit `agent_config.json`.

---

## Testing

```bash
cargo test
```

Menjalankan 20 unit test + 1 integration test (mock HTTP server).

---

## License

Internal & Proprietary — PT Gerlink Utama Mandiri & Fathforce Ecosystem.
