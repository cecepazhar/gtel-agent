use chrono::Utc;
use std::net::SocketAddr;
use telemetry_agent::client::TelemetryClient;
use telemetry_agent::collector::{self, ServerResponse};
use telemetry_agent::config::Config;
use telemetry_agent::db::{seed_sample_reporting_db, QueueDb};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn test_end_to_end_dispatch_and_sync() {
    // 1. Setup local mock HTTP server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server_url = format!("http://{}/v1/update/check", addr);

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]);

            // Assert cloaked User-Agent header is present
            assert!(req_str.contains("User-Agent: GerlinkUpdateService/1.2") || req_str.contains("user-agent: GerlinkUpdateService/1.2"));

            let body = r#"{"status":"success","update_available":false,"latest_version":"1.2.0","server_timestamp":"2026-10-28T03:00:00Z"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    // 2. Setup temp DBs
    let temp_dir = std::env::temp_dir();
    let ts = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let reporting_db = temp_dir.join(format!("test_e2e_rep_{}.db", ts));
    let queue_db_path = temp_dir.join(format!("test_e2e_queue_{}.db", ts));

    seed_sample_reporting_db(reporting_db.to_str().unwrap()).unwrap();

    let mut cfg = Config::default();
    cfg.server_url = server_url;
    cfg.db_path = reporting_db.to_str().unwrap().to_string();
    cfg.queue_db_path = queue_db_path.to_str().unwrap().to_string();

    let queue_db = QueueDb::new(&cfg.queue_db_path).unwrap();
    let client = TelemetryClient::new(&cfg.server_url).unwrap();

    // 3. Collect
    let payload = collector::collect_telemetry(&cfg, "DEV-TEST-HWID").unwrap();
    assert_eq!(payload.usage_metrics.usage_count, 142);
    assert_eq!(payload.productivity_metrics.total_patients, 420);

    let json = serde_json::to_string(&payload).unwrap();
    let queue_id = queue_db.enqueue(&json).unwrap();

    // 4. Dispatch
    let resp: ServerResponse = client.send_payload(&payload).await.expect("Dispatch should succeed");
    assert_eq!(resp.status, "success");
    assert!(!resp.update_available);
    assert_eq!(resp.latest_version, "1.2.0");

    queue_db.mark_synced(queue_id).unwrap();

    // 5. Verify stats
    let stats = queue_db.get_stats().unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.synced, 1);
    assert_eq!(stats.unsynced, 0);

    let _ = std::fs::remove_file(reporting_db);
    let _ = std::fs::remove_file(queue_db_path);
}
