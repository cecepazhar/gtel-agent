use crate::collector::{ServerResponse, TelemetryPayload};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use std::time::Duration;

pub const CLOAKED_USER_AGENT: &str = "GerlinkUpdateService/1.2";

#[derive(Clone)]
pub struct TelemetryClient {
    client: reqwest::Client,
    server_url: String,
}

impl TelemetryClient {
    pub fn new(server_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(CLOAKED_USER_AGENT),
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            client,
            server_url: server_url.to_string(),
        })
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Dispatches structured TelemetryPayload to update-checker endpoint
    pub async fn send_payload(&self, payload: &TelemetryPayload) -> Result<ServerResponse, String> {
        let json_body = serde_json::to_string(payload)
            .map_err(|e| format!("Serialization error: {}", e))?;
        self.send_raw_json(&json_body).await
    }

    /// Dispatches raw JSON string payload (used when replaying offline queue)
    pub async fn send_raw_json(&self, raw_json: &str) -> Result<ServerResponse, String> {
        let response = self
            .client
            .post(&self.server_url)
            .body(raw_json.to_string())
            .send()
            .await
            .map_err(|e| format!("HTTP request error: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown server error body".to_string());
            return Err(format!(
                "Server returned HTTP {} error: {}",
                status, error_text
            ));
        }

        let body_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        let server_resp: ServerResponse = serde_json::from_str(&body_text)
            .map_err(|e| format!("Invalid JSON response from server: {} (body: {})", e, body_text))?;

        Ok(server_resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_init() {
        let client = TelemetryClient::new("https://apig.fathforce.com/v1/update/check");
        assert!(client.is_ok());
        let c = client.unwrap();
        assert_eq!(c.server_url(), "https://apig.fathforce.com/v1/update/check");
    }
}
