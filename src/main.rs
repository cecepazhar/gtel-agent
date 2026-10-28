use clap::{Parser, Subcommand};
use std::path::Path;
use std::time::Duration;
use telemetry_agent::client::TelemetryClient;
use telemetry_agent::collector;
use telemetry_agent::config::Config;
use telemetry_agent::db::{seed_sample_reporting_db, QueueDb, ReportingDbReader};
use telemetry_agent::hwid::HardwareFingerprint;

#[derive(Parser)]
#[command(name = "telemetry-agent")]
#[command(author = "PT Gerlink Utama Mandiri <rnd@gerlink.id>")]
#[command(version = "1.2.0")]
#[command(about = "Gerlink TelemetryAgent background client for medical reporting stations")]
struct Cli {
    #[arg(short, long, global = true, help = "Path to optional config JSON file")]
    config: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Start daemon loop to periodically collect and dispatch telemetry")]
    Run {
        #[arg(
            short,
            long,
            help = "Interval between sync cycles in seconds (default: config value or 300)"
        )]
        interval: Option<u64>,
    },

    #[command(about = "Execute immediate single probe & dispatch")]
    SendOnce,

    #[command(
        about = "Display agent status, hardware fingerprint, queue metrics and configuration"
    )]
    Status,

    #[command(about = "Verify local SQLite reporting database connectivity")]
    TestDb {
        #[arg(short, long, help = "Custom SQLite reporting DB path to inspect")]
        path: Option<String>,

        #[arg(
            long,
            help = "Seed sample reporting database schema and test clinical records"
        )]
        seed: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = Config::load_or_default(cli.config.as_deref());

    match cli.command {
        Commands::Run { interval } => {
            let tick_secs = interval.unwrap_or(config.interval_secs);
            cmd_run(&config, tick_secs).await?;
        }
        Commands::SendOnce => {
            cmd_send_once(&config).await?;
        }
        Commands::Status => {
            cmd_status(&config)?;
        }
        Commands::TestDb { path, seed } => {
            cmd_test_db(&config, path.as_deref(), seed)?;
        }
    }

    Ok(())
}

async fn cmd_run(config: &Config, interval_secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let effective_id = config.get_effective_device_id(&hw.hwid);
    let queue_db = QueueDb::new(&config.queue_db_path)?;
    let client = TelemetryClient::new(&config.server_url)?;

    println!("============================================================");
    println!("     Gerlink TelemetryAgent Client v1.2.0 (Daemon Loop)     ");
    println!("============================================================");
    println!("Device ID      : {}", effective_id);
    println!("Software       : {} v{}", config.software_name, config.software_version);
    println!("Hospital       : {}", config.hospital_name);
    println!("Reporting DB   : {}", config.db_path);
    println!("Queue DB       : {}", config.queue_db_path);
    println!("Target Server  : {}", config.server_url);
    println!("Sync Interval  : {} seconds", interval_secs);
    println!("============================================================");
    println!("Press Ctrl+C to stop daemon.\n");

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[SHUTDOWN] Received Ctrl+C termination signal. Exiting gracefully.");
                break;
            }
            _ = interval.tick() => {
                let timestamp = chrono::Utc::now().to_rfc3339();
                println!("[{}] Starting telemetry probe and sync cycle...", timestamp);

                // 1. Drain offline queue first
                replay_offline_queue(&queue_db, &client).await;

                // 2. Collect current metrics
                match collector::collect_telemetry(config, &hw.hwid) {
                    Ok(payload) => {
                        let json = match serde_json::to_string(&payload) {
                            Ok(j) => j,
                            Err(e) => {
                                eprintln!("[ERROR] JSON serialization failed: {}", e);
                                continue;
                            }
                        };

                        // Store in queue table (offline resilience)
                        let queue_id = match queue_db.enqueue(&json) {
                            Ok(id) => id,
                            Err(e) => {
                                eprintln!("[ERROR] Failed to save payload to local queue DB: {}", e);
                                continue;
                            }
                        };

                        // 3. Dispatch to cloaked update-checker endpoint
                        match client.send_payload(&payload).await {
                            Ok(resp) => {
                                let _ = queue_db.mark_synced(queue_id);
                                println!(
                                    "[ONLINE] Synced successfully (Queue ID: {}). Server Status: '{}', Update Available: {}, Latest: '{}'",
                                    queue_id, resp.status, resp.update_available, resp.latest_version
                                );
                            }
                            Err(err) => {
                                let _ = queue_db.mark_failed(queue_id, &err);
                                eprintln!(
                                    "[OFFLINE] Network dispatch failed: {}. Queued in local SQLite (Queue ID: {}) for retry.",
                                    err, queue_id
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ERROR] Failed to probe telemetry data: {}", e);
                    }
                }
            }
        }
    }

    Ok(())
}

async fn cmd_send_once(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let effective_id = config.get_effective_device_id(&hw.hwid);
    let queue_db = QueueDb::new(&config.queue_db_path)?;
    let client = TelemetryClient::new(&config.server_url)?;

    println!("Probing Gerlink TelemetryAgent metrics (single dispatch)...");
    println!("Device ID      : {}", effective_id);
    println!("Software       : {} v{}", config.software_name, config.software_version);
    println!("Target Server  : {}", config.server_url);

    // Replay any earlier unsynced items
    replay_offline_queue(&queue_db, &client).await;

    // Collect telemetry
    let payload = collector::collect_telemetry(config, &hw.hwid)?;
    let json = serde_json::to_string_pretty(&payload)?;
    println!("\nCollected Payload:\n{}", json);

    let queue_id = queue_db.enqueue(&json)?;
    println!("\nEnqueued to local offline queue with ID: {}", queue_id);

    print!("Dispatching to cloaked endpoint {} ... ", config.server_url);
    match client.send_payload(&payload).await {
        Ok(resp) => {
            queue_db.mark_synced(queue_id)?;
            println!("SUCCESS!");
            println!("Server Response:");
            println!("  Status           : {}", resp.status);
            println!("  Update Available : {}", resp.update_available);
            println!("  Latest Version   : {}", resp.latest_version);
            if let Some(ts) = resp.server_timestamp {
                println!("  Server Timestamp : {}", ts);
            }
        }
        Err(err) => {
            queue_db.mark_failed(queue_id, &err)?;
            println!("OFFLINE / FAILED");
            println!(
                "Note: Telemetry payload saved safely to local queue (ID: {}). It will be re-sent automatically on next run.",
                queue_id
            );
            println!("Error detail: {}", err);
        }
    }

    Ok(())
}

fn cmd_status(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let effective_id = config.get_effective_device_id(&hw.hwid);
    let queue_db = QueueDb::new(&config.queue_db_path)?;
    let stats = queue_db.get_stats()?;

    println!("============================================================");
    println!("               Gerlink TelemetryAgent Status                ");
    println!("============================================================");
    println!("Hardware Fingerprint (HWID):");
    println!("  Effective Device ID : {}", effective_id);
    println!("  Generated HWID      : {}", hw.hwid);
    println!("  CPU                 : {}", hw.cpu);
    println!("  Motherboard UUID    : {}", hw.motherboard_uuid);
    println!("  Primary MAC Address : {}", hw.mac_address);
    println!("  Primary Disk Serial : {}", hw.disk_serial);
    println!("  Raw Fingerprint     : {}", hw.raw_fingerprint);
    println!();
    println!("Target Endpoint:");
    println!("  Server URL          : {}", config.server_url);
    println!("  User-Agent          : {}", telemetry_agent::client::CLOAKED_USER_AGENT);
    println!();
    println!("Software & Faskes Profile:");
    println!("  Software Name       : {}", config.software_name);
    println!("  Software Version    : {}", config.software_version);
    println!("  Hospital Name       : {}", config.hospital_name);
    println!("  Region / City       : {}, {}", config.region, config.city);
    println!("  Room Name           : {}", config.room_name);
    println!("  Coordinates         : [{}, {}]", config.latitude, config.longitude);
    println!();
    println!("Reporting Database:");
    println!("  Path                : {}", config.db_path);
    println!("  File Exists         : {}", Path::new(&config.db_path).exists());
    println!("  Mock Fallback       : {}", config.enable_mock_if_missing);
    println!();
    println!("Local Offline Queue (SQLite):");
    println!("  Queue DB Path       : {}", config.queue_db_path);
    println!("  Total Enqueued      : {}", stats.total);
    println!("  Synced Successfully : {}", stats.synced);
    println!("  Unsynced (Pending)  : {}", stats.unsynced);
    if let Some(ref attempt) = stats.last_attempt {
        println!("  Last Attempt Time   : {}", attempt);
    }
    println!("============================================================");

    Ok(())
}

fn cmd_test_db(
    config: &Config,
    custom_path: Option<&str>,
    seed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = custom_path.unwrap_or(&config.db_path);

    println!("============================================================");
    println!("          Gerlink Telemetry Reporting DB Inspector          ");
    println!("============================================================");
    println!("Target SQLite Database: {}", db_path);

    if seed {
        println!("Seeding sample realistic reporting schema & records...");
        seed_sample_reporting_db(db_path)?;
        println!("Database seeded successfully at '{}'!\n", db_path);
    }

    if !Path::new(db_path).exists() {
        println!("Status: NOT FOUND");
        println!(
            "Database file does not exist. Use `--seed` to generate a sample database for testing:"
        );
        println!("  telemetry-agent test-db --path {} --seed", db_path);
        return Ok(());
    }

    match ReportingDbReader::read(db_path) {
        Ok(data) => {
            println!("Status: CONNECTED & READ SUCCESSFUL\n");
            println!("Usage Metrics:");
            println!("  Usage Count (App Launches) : {}", data.usage_count);
            println!("  Software Active Hours      : {:.2} hrs", data.software_active_hours);
            println!("  Light Source Runtime Hours : {:.2} hrs", data.light_source_hours);
            println!();
            println!("Productivity Metrics (Non-PII):");
            println!("  Total Patients Registered  : {}", data.total_patients);
            println!("  Total Doctors / Operators  : {}", data.total_doctors);
            println!("  Total Examinations Done    : {}", data.total_examinations);
            println!("  Total Medical Reports Gen  : {}", data.total_reports_generated);
            println!();
            println!("Detected Errors Count: {}", data.detected_errors.len());
            for (idx, err) in data.detected_errors.iter().enumerate() {
                println!(
                    "  [{}] ({}) [{}] {}",
                    idx + 1,
                    err.timestamp,
                    err.module,
                    err.error_message
                );
            }
        }
        Err(e) => {
            println!("Status: FAILED TO READ DATA");
            println!("Error: {}", e);
        }
    }
    println!("============================================================");

    Ok(())
}

async fn replay_offline_queue(queue_db: &QueueDb, client: &TelemetryClient) {
    let unsynced = match queue_db.get_unsynced() {
        Ok(list) => list,
        Err(_) => return,
    };

    if unsynced.is_empty() {
        return;
    }

    println!("[QUEUE] Found {} unsynced offline records. Attempting replay...", unsynced.len());

    for (id, raw_json) in unsynced {
        match client.send_raw_json(&raw_json).await {
            Ok(_) => {
                let _ = queue_db.mark_synced(id);
                println!("[QUEUE] Offline record #{} synced successfully.", id);
            }
            Err(e) => {
                let _ = queue_db.mark_failed(id, &e);
                println!("[QUEUE] Replay paused: network error on record #{}: {}", id, e);
                break; // Stop replaying on network failure to avoid hammer
            }
        }
    }
}
