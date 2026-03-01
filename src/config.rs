use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub delivery: DeliveryConfig,
    #[serde(default)]
    pub persistence: PersistenceConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DaemonConfig {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
}

fn default_listen() -> SocketAddr {
    "127.0.0.1:3100".parse().unwrap()
}

fn default_tick_ms() -> u64 {
    100
}

fn default_state_dir() -> PathBuf {
    PathBuf::from("./state")
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            tick_ms: default_tick_ms(),
            state_dir: default_state_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DeliveryConfig {
    #[serde(default = "default_callback_url")]
    pub default_callback_url: String,
    #[serde(default)]
    pub webhook_token: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub retry_backoff_ms: Vec<u64>,
    #[serde(default = "default_outbox_file")]
    pub outbox_file: String,
}

fn default_callback_url() -> String {
    "http://127.0.0.1:18789/hooks/wake".to_string()
}

fn default_timeout_secs() -> u64 {
    10
}

fn default_retry_attempts() -> u32 {
    3
}

fn default_retry_backoff_ms() -> Vec<u64> {
    vec![1000, 5000, 30000]
}

fn default_outbox_file() -> String {
    "outbox.jsonl".to_string()
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            default_callback_url: default_callback_url(),
            webhook_token: None,
            timeout_secs: default_timeout_secs(),
            retry_attempts: default_retry_attempts(),
            retry_backoff_ms: default_retry_backoff_ms(),
            outbox_file: default_outbox_file(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PersistenceConfig {
    #[serde(default = "default_snapshot_every_ops")]
    pub snapshot_every_ops: usize,
    #[serde(default = "default_snapshot_every_secs")]
    pub snapshot_every_secs: u64,
}

fn default_snapshot_every_ops() -> usize {
    1000
}

fn default_snapshot_every_secs() -> u64 {
    300
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            snapshot_every_ops: default_snapshot_every_ops(),
            snapshot_every_secs: default_snapshot_every_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LimitsConfig {
    #[serde(default = "default_max_timers")]
    pub max_timers: usize,
    #[serde(default = "default_max_payload_bytes")]
    pub max_payload_bytes: usize,
    #[serde(default = "default_max_tags")]
    pub max_tags: usize,
    #[serde(default = "default_min_fire_ms")]
    pub min_fire_ms: u64,
}

fn default_max_timers() -> usize {
    100000
}

fn default_max_payload_bytes() -> usize {
    65536
}

fn default_max_tags() -> usize {
    20
}

fn default_min_fire_ms() -> u64 {
    100
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_timers: default_max_timers(),
            max_payload_bytes: default_max_payload_bytes(),
            max_tags: default_max_tags(),
            min_fire_ms: default_min_fire_ms(),
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
