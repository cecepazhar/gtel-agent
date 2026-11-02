use clap::{Parser, Subcommand};
use std::path::Path;
use std::time::Duration;
use telemetry_agent::client::TelemetryClient;
use telemetry_agent::collector;
use telemetry_agent::config::{Config, Profile, SoftwareName};
use telemetry_agent::db::{seed_sample_reporting_db, QueueDb, ReportingDbReader};
use telemetry_agent::health::{calculate_status, collect_health_report_for_profile};
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
    #[command(about = "Start daemon loop to periodically collect and dispatch telemetry for all profiles")]
    Run {
        #[arg(
            short,
            long,
            help = "Interval between sync cycles in seconds (default: config value or 300)"
        )]
        interval: Option<u64>,
    },

    #[command(about = "Execute immediate single probe & dispatch for all profiles (or a specific brand)")]
    SendOnce {
        #[arg(short, long, help = "Filter to a specific brand: GLESCOPE, ANTECH, EASTTECH, ZEUS")]
        brand: Option<String>,
    },

    #[command(
        about = "Display agent status, hardware fingerprint, queue metrics and all brand profiles"
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

    #[command(about = "Display current application health report for each brand")]
    Health,

    #[command(about = "Generate a default multi-brand agent_config.json containing Glescope, Antech, Easttech, and Zeus")]
    InitConfig {
        #[arg(short, long, default_value = "agent_config.json", help = "Output filename")]
        output: String,
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
        Commands::SendOnce { brand } => {
            cmd_send_once(&config, brand.as_deref()).await?;
        }
        Commands::Status => {
            cmd_status(&config)?;
        }
        Commands::TestDb { path, seed } => {
            cmd_test_db(&config, path.as_deref(), seed)?;
        }
        Commands::Health => {
            cmd_health(&config).await?;
        }
        Commands::InitConfig { output } => {
            cmd_init_config(&output)?;
        }
    }

    Ok(())
}

async fn cmd_run(config: &Config, interval_secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let profiles = config.active_profiles();
    let client = TelemetryClient::new(&config.server_url)?;

    println!("============================================================");
    println!("  Gerlink TelemetryAgent Multi-Brand Daemon (v1.2.0)        ");
    println!("============================================================");
    println!("HWID Fingerprint : {}", hw.hwid);
    println!("Hospital         : {}", config.hospital_name);
    println!("Target Server    : {}", config.server_url);
    println!("Sync Interval    : {} seconds", interval_secs);
    println!("Tracked Brands   : {} profiles active", profiles.len());
    for (i, p) in profiles.iter().enumerate() {
        let eff_id = config.get_effective_device_id(p, &hw.hwid);
        println!(
            "  [{}] {:<9} v{:<6} | DeviceID: {} | DB: {}",
            i + 1,
            p.label(),
            p.software_version,
            eff_id,
            p.db_path
        );
    }
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
                println!("[{}] Starting multi-brand telemetry cycle...", timestamp);

                for profile in &profiles {
                    let brand_label = profile.label();
                    let queue_db = match QueueDb::new(&profile.queue_db_path) {
                        Ok(db) => db,
                        Err(e) => {
                            eprintln!("[{}] [{}] Failed to open queue DB ({}): {}", timestamp, brand_label, profile.queue_db_path, e);
                            continue;
                        }
                    };

                    // 1. Drain offline queue first for this brand
                    replay_offline_queue(&queue_db, &client, &brand_label).await;

                    // 2. Probe current metrics
                    match collector::collect_telemetry_for_profile(config, profile, &hw.hwid) {
                        Ok(payload) => {
                            let json = match serde_json::to_string(&payload) {
                                Ok(j) => j,
                                Err(e) => {
                                    eprintln!("[{}] [{}] JSON serialization failed: {}", timestamp, brand_label, e);
                                    continue;
                                }
                            };

                            let queue_id = match queue_db.enqueue(&json) {
                                Ok(id) => id,
                                Err(e) => {
                                    eprintln!("[{}] [{}] Enqueue failed: {}", timestamp, brand_label, e);
                                    continue;
                                }
                            };

                            // 3. Dispatch to server
                            match client.send_payload(&payload).await {
                                Ok(resp) => {
                                    let _ = queue_db.mark_synced(queue_id);
                                    println!(
                                        "[{}] [{}] ONLINE: Synced #{} (DevID: {}). Status: '{}', Update: {}",
                                        timestamp,
                                        brand_label,
                                        queue_id,
                                        payload.device_id,
                                        resp.status,
                                        resp.update_available
                                    );
                                }
                                Err(err) => {
                                    let _ = queue_db.mark_failed(queue_id, &err);
                                    eprintln!(
                                        "[{}] [{}] OFFLINE: #{} queued for retry. Reason: {}",
                                        timestamp,
                                        brand_label,
                                        queue_id,
                                        err
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[{}] [{}] Probe failed: {}", timestamp, brand_label, e);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn cmd_send_once(config: &Config, brand_filter: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let all_profiles = config.active_profiles();
    let client = TelemetryClient::new(&config.server_url)?;

    let target_profiles: Vec<&Profile> = if let Some(bf) = brand_filter {
        let needle = bf.trim().to_uppercase();
        let filtered: Vec<&Profile> = all_profiles
            .iter()
            .filter(|p| p.label().to_uppercase() == needle)
            .collect();
        if filtered.is_empty() {
            eprintln!(
                "Error: Brand '{}' not found in active profiles. Available: {}",
                bf,
                all_profiles.iter().map(|p| p.label()).collect::<Vec<_>>().join(", ")
            );
            return Ok(());
        }
        filtered
    } else {
        all_profiles.iter().collect()
    };

    println!("Executing single probe for {} profile(s)...", target_profiles.len());

    for profile in target_profiles {
        let brand_label = profile.label();
        let effective_id = config.get_effective_device_id(profile, &hw.hwid);
        let queue_db = QueueDb::new(&profile.queue_db_path)?;

        println!("\n--- Brand: {} (v{}) ---", brand_label, profile.software_version);
        println!("Device ID      : {}", effective_id);
        println!("Reporting DB   : {}", profile.db_path);
        println!("Queue DB       : {}", profile.queue_db_path);

        replay_offline_queue(&queue_db, &client, &brand_label).await;

        let payload = collector::collect_telemetry_for_profile(config, profile, &hw.hwid)?;
        let json = serde_json::to_string_pretty(&payload)?;
        println!("Payload snippet: {} bytes", json.len());

        let queue_id = queue_db.enqueue(&json)?;
        print!("Dispatching to {} ... ", config.server_url);

        match client.send_payload(&payload).await {
            Ok(resp) => {
                queue_db.mark_synced(queue_id)?;
                println!("SUCCESS!");
                println!("  Status           : {}", resp.status);
                println!("  Update Available : {}", resp.update_available);
                println!("  Latest Version   : {}", resp.latest_version);
            }
            Err(err) => {
                queue_db.mark_failed(queue_id, &err)?;
                println!("OFFLINE / FAILED (saved to queue #{})", queue_id);
                println!("  Error: {}", err);
            }
        }
    }

    Ok(())
}

fn cmd_status(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let hw = HardwareFingerprint::generate();
    let profiles = config.active_profiles();

    println!("============================================================");
    println!("       Gerlink TelemetryAgent Status (Multi-Brand)          ");
    println!("============================================================");
    println!("Hardware Fingerprint (HWID):");
    println!("  Generated HWID      : {}", hw.hwid);
    println!("  CPU                 : {}", hw.cpu);
    println!("  Motherboard UUID    : {}", hw.motherboard_uuid);
    println!("  Primary MAC Address : {}", hw.mac_address);
    println!("  Primary Disk Serial : {}", hw.disk_serial);
    println!();
    println!("Target Server & Faskes Profile:");
    println!("  Server URL          : {}", config.server_url);
    println!("  User-Agent          : {}", telemetry_agent::client::CLOAKED_USER_AGENT);
    println!("  Hospital Name       : {}", config.hospital_name);
    println!("  Region / City       : {}, {}", config.region, config.city);
    println!("  Room Name           : {}", config.room_name);
    println!("  Coordinates         : [{}, {}]", config.latitude, config.longitude);
    println!("  Interval (secs)     : {}", config.interval_secs);
    println!("  Mock Fallback       : {}", config.enable_mock_if_missing);
    println!();
    println!("Tracked Brand Profiles ({} total):", profiles.len());
    for (i, p) in profiles.iter().enumerate() {
        let eff_id = config.get_effective_device_id(p, &hw.hwid);
        let exists = Path::new(&p.db_path).exists();
        let queue_stats = match QueueDb::new(&p.queue_db_path) {
            Ok(q) => match q.get_stats() {
                Ok(s) => format!("Queue: {} total, {} synced, {} pending", s.total, s.synced, s.unsynced),
                Err(_) => "Queue stats error".to_string(),
            },
            Err(_) => "Queue DB not initialized".to_string(),
        };

        println!("  [{}] {} (v{})", i + 1, p.label(), p.software_version);
        println!("      Effective Device ID : {}", eff_id);
        println!("      Reporting DB        : {} (exists: {})", p.db_path, exists);
        println!("      Queue DB            : {} ({})", p.queue_db_path, queue_stats);
    }
    println!("============================================================");

    Ok(())
}

fn cmd_test_db(
    config: &Config,
    custom_path: Option<&str>,
    seed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = if let Some(p) = custom_path {
        p.to_string()
    } else {
        let first = config.active_profiles();
        first
            .first()
            .map(|p| p.db_path.clone())
            .unwrap_or_else(|| r"D:\GLESCOPE_DB\telemetry.db".to_string())
    };

    println!("============================================================");
    println!("          Gerlink Telemetry Reporting DB Inspector          ");
    println!("============================================================");
    println!("Target SQLite Database: {}", db_path);

    if seed {
        println!("Seeding sample realistic reporting schema & records...");
        seed_sample_reporting_db(&db_path)?;
        println!("Database seeded successfully at '{}'!\n", db_path);
    }

    if !Path::new(&db_path).exists() {
        println!("Status: NOT FOUND");
        println!(
            "Database file does not exist. Use `--seed` to generate a sample database for testing:"
        );
        println!("  telemetry-agent test-db --path \"{}\" --seed", db_path);
        return Ok(());
    }

    match ReportingDbReader::read(&db_path) {
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

async fn cmd_health(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let profiles = config.active_profiles();

    println!("============================================================");
    println!("     Gerlink TelemetryAgent Health Report (Multi-Brand)     ");
    println!("============================================================");

    for (i, profile) in profiles.iter().enumerate() {
        let queue_db = QueueDb::new(&profile.queue_db_path)?;

        let detected_errors = if Path::new(&profile.db_path).exists() {
            match ReportingDbReader::read(&profile.db_path) {
                Ok(rep) => rep
                    .detected_errors
                    .into_iter()
                    .map(|e| collector::DetectedError {
                        timestamp: e.timestamp,
                        module: e.module,
                        error_message: e.error_message,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        } else if config.enable_mock_if_missing {
            collector::generate_mock_metrics().2
        } else {
            Vec::new()
        };

        let report = collect_health_report_for_profile(config, profile, &queue_db, &detected_errors).await?;
        let status = calculate_status(&report);

        println!("Brand [{}]: {} (v{})", i + 1, profile.label(), profile.software_version);
        println!("  Overall Status     : {}", status);
        println!("  Uptime             : {:.2}s", report.agent_uptime_seconds);
        println!("  Memory RSS         : {:.2} MB", report.memory_rss_mb);
        println!("  CPU Usage          : {:.2} %", report.cpu_pct);
        println!("  Queue Depth        : {}", report.queue_depth);
        println!("  DB Size            : {:.2} MB", report.db_size_mb);
        println!("  Errors (24h)       : {}", report.error_count_24h);
        println!("  Connected to Server: {}", report.is_connected_to_server);
        if let Some(ref last) = report.last_error_message {
            println!("  Last Error         : {}", last);
        }
        println!();
    }
    println!("============================================================");

    Ok(())
}

fn cmd_init_config(output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Config::default();
    cfg.profiles = vec![
        Profile::new_software(SoftwareName::Glescope),
        Profile::new_software(SoftwareName::Antech),
        Profile::new_software(SoftwareName::Easttech),
        Profile::new_software(SoftwareName::Zeus),
    ];

    let json = serde_json::to_string_pretty(&cfg)?;
    std::fs::write(output_path, json)?;
    println!(
        "Successfully created multi-brand template configuration file at '{}'.",
        output_path
    );
    println!("It tracks 4 brands: Glescope, Antech, Easttech, and Zeus.");

    Ok(())
}

async fn replay_offline_queue(queue_db: &QueueDb, client: &TelemetryClient, label: &str) {
    let unsynced = match queue_db.get_unsynced() {
        Ok(list) => list,
        Err(_) => return,
    };

    if unsynced.is_empty() {
        return;
    }

    println!("[{}] [QUEUE] Found {} unsynced offline records. Attempting replay...", label, unsynced.len());

    for (id, raw_json) in unsynced {
        match client.send_raw_json(&raw_json).await {
            Ok(_) => {
                let _ = queue_db.mark_synced(id);
                println!("[{}] [QUEUE] Offline record #{} synced successfully.", label, id);
            }
            Err(e) => {
                let _ = queue_db.mark_failed(id, &e);
                println!("[{}] [QUEUE] Replay paused: network error on record #{}: {}", label, id, e);
                break;
            }
        }
    }
}
