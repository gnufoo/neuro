use crate::config::DeliveryConfig;
use crate::types::{FireEvent, OutboxEntry};
use chrono::Utc;
use reqwest::Client;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

pub struct Delivery {
    client: Client,
    config: DeliveryConfig,
}

impl Delivery {
    pub fn new(config: DeliveryConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("failed to build HTTP client");

        Self { client, config }
    }

    pub async fn deliver(
        &self,
        event: &FireEvent,
        url: &str,
        outbox_path: &Path,
    ) -> Result<(), DeliveryError> {
        let mut last_error_msg = String::new();

        for attempt in 0..self.config.retry_attempts {
            if attempt > 0 {
                let backoff = self
                    .config
                    .retry_backoff_ms
                    .get((attempt - 1) as usize)
                    .copied()
                    .unwrap_or(30000);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
            }

            let result = self.deliver_once(event, url).await;

            match result {
                Ok(()) => {
                    info!(
                        "webhook delivered successfully: timer_id={}",
                        event.timer_id
                    );
                    return Ok(());
                }
                Err(e) => {
                    last_error_msg = e.to_string();
                    warn!(
                        "webhook delivery failed (attempt {}/{}): {}",
                        attempt + 1,
                        self.config.retry_attempts,
                        e
                    );

                    // Don't retry on 4xx
                    if let DeliveryError::HttpError(status) = &e {
                        if (400..500).contains(status) {
                            warn!("4xx error, not retrying");
                            break;
                        }
                    }
                }
            }
        }

        // All retries failed, write to outbox
        let error_msg = if last_error_msg.is_empty() {
            "unknown error".to_string()
        } else {
            last_error_msg
        };

        self.write_outbox(event, &error_msg, self.config.retry_attempts, outbox_path)
            .await?;

        Err(DeliveryError::Outboxed(error_msg))
    }

    async fn deliver_once(&self, event: &FireEvent, url: &str) -> Result<(), DeliveryError> {
        // Format as OpenClaw /hooks/wake payload
        let wake_text = format!(
            "[Neuro Timer Fired] timer_id={} tags={} priority={} fire_count={} payload={}",
            event.timer_id,
            event.tags.join(","),
            event.priority,
            event.fire_count,
            serde_json::to_string(&event.payload).unwrap_or_default()
        );

        let wake_payload = json!({
            "text": wake_text,
            "mode": "now"
        });

        let mut req = self.client.post(url);

        // Add auth token if configured
        if let Some(ref token) = self.config.webhook_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let response = req
            .json(&wake_payload)
            .send()
            .await
            .map_err(|e| DeliveryError::NetworkError(Arc::new(e)))?;

        let status = response.status();

        if status.is_success() {
            Ok(())
        } else if status.is_server_error() {
            Err(DeliveryError::HttpError(status.as_u16()))
        } else if status.is_client_error() {
            Err(DeliveryError::HttpError(status.as_u16()))
        } else {
            Err(DeliveryError::HttpError(status.as_u16()))
        }
    }

    async fn write_outbox(
        &self,
        event: &FireEvent,
        error: &str,
        attempts: u32,
        outbox_path: &Path,
    ) -> Result<(), DeliveryError> {
        let entry = OutboxEntry {
            event: event.clone(),
            delivery_error: error.to_string(),
            attempts,
            outboxed_at: Utc::now(),
        };

        let line = serde_json::to_string(&entry).map_err(|e| DeliveryError::SerializeError(Arc::new(e)))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(outbox_path)
            .await
            .map_err(|e| DeliveryError::IoError(Arc::new(e)))?;

        file.write_all(line.as_bytes()).await.map_err(|e| DeliveryError::IoError(Arc::new(e)))?;
        file.write_all(b"\n").await.map_err(|e| DeliveryError::IoError(Arc::new(e)))?;

        Ok(())
    }

    pub fn default_url(&self) -> &str {
        &self.config.default_callback_url
    }

    pub fn outbox_path(&self, state_dir: &Path) -> PathBuf {
        state_dir.join(&self.config.outbox_file)
    }
}

#[derive(Debug)]
pub enum DeliveryError {
    NetworkError(Arc<reqwest::Error>),
    HttpError(u16),
    SerializeError(Arc<serde_json::Error>),
    IoError(Arc<std::io::Error>),
    Outboxed(String),
}

impl Clone for DeliveryError {
    fn clone(&self) -> Self {
        match self {
            DeliveryError::NetworkError(e) => DeliveryError::NetworkError(e.clone()),
            DeliveryError::HttpError(code) => DeliveryError::HttpError(*code),
            DeliveryError::SerializeError(e) => DeliveryError::SerializeError(e.clone()),
            DeliveryError::IoError(e) => DeliveryError::IoError(e.clone()),
            DeliveryError::Outboxed(s) => DeliveryError::Outboxed(s.clone()),
        }
    }
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeliveryError::NetworkError(e) => write!(f, "network error: {}", e),
            DeliveryError::HttpError(code) => write!(f, "HTTP error: {}", code),
            DeliveryError::SerializeError(e) => write!(f, "serialization error: {}", e),
            DeliveryError::IoError(e) => write!(f, "I/O error: {}", e),
            DeliveryError::Outboxed(s) => write!(f, "outboxed: {}", s),
        }
    }
}

impl std::error::Error for DeliveryError {}
