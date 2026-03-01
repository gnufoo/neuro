use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timer {
    pub id: String,
    pub fire_at: DateTime<Utc>,
    pub priority: u8,
    pub payload: serde_json::Value,
    pub repeat_ms: Option<u64>,
    pub max_fires: Option<u32>,
    pub ttl: Option<DateTime<Utc>>,
    pub fire_count: u32,
    pub callback_url: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Timer {
    pub fn new(
        id: String,
        fire_at: DateTime<Utc>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id,
            fire_at,
            priority: 128,
            payload,
            repeat_ms: None,
            max_fires: None,
            ttl: None,
            fire_count: 0,
            callback_url: None,
            tags: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FireEvent {
    pub timer_id: String,
    pub fired_at: DateTime<Utc>,
    pub fire_count: u32,
    pub priority: u8,
    pub payload: serde_json::Value,
    pub tags: Vec<String>,
    pub will_repeat: bool,
    pub next_fire_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub late: bool,
}

impl FireEvent {
    pub fn from_timer(timer: &Timer, fired_at: DateTime<Utc>, late: bool) -> Self {
        let will_repeat = timer.repeat_ms.is_some()
            && timer.max_fires.map_or(true, |max| timer.fire_count + 1 < max);

        let next_fire_at = if will_repeat {
            timer.repeat_ms.map(|ms| timer.fire_at + chrono::Duration::milliseconds(ms as i64))
        } else {
            None
        };

        Self {
            timer_id: timer.id.clone(),
            fired_at,
            fire_count: timer.fire_count,
            priority: timer.priority,
            payload: timer.payload.clone(),
            tags: timer.tags.clone(),
            will_repeat,
            next_fire_at,
            late,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerSummary {
    pub id: String,
    pub fire_at: DateTime<Utc>,
    pub priority: u8,
    pub fire_count: u32,
    pub repeat_ms: Option<u64>,
    pub tags: Vec<String>,
    pub payload_keys: Vec<String>,
}

impl From<&Timer> for TimerSummary {
    fn from(timer: &Timer) -> Self {
        let payload_keys = if let Some(obj) = timer.payload.as_object() {
            obj.keys().cloned().collect()
        } else {
            Vec::new()
        };

        Self {
            id: timer.id.clone(),
            fire_at: timer.fire_at,
            priority: timer.priority,
            fire_count: timer.fire_count,
            repeat_ms: timer.repeat_ms,
            tags: timer.tags.clone(),
            payload_keys,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum WalEntry {
    #[serde(rename = "schedule")]
    Schedule { timer: Timer, ts: DateTime<Utc> },
    #[serde(rename = "cancel")]
    Cancel { id: String, ts: DateTime<Utc> },
    #[serde(rename = "cancel_tag")]
    CancelTag { tag: String, ts: DateTime<Utc> },
    #[serde(rename = "update")]
    Update {
        id: String,
        fields: HashMap<String, serde_json::Value>,
        ts: DateTime<Utc>,
    },
    #[serde(rename = "fire")]
    Fire { id: String, fire_count: u32, ts: DateTime<Utc> },
}

impl WalEntry {
    pub fn schedule(timer: Timer) -> Self {
        WalEntry::Schedule {
            timer,
            ts: Utc::now(),
        }
    }

    pub fn cancel(id: String) -> Self {
        WalEntry::Cancel { id, ts: Utc::now() }
    }

    pub fn cancel_tag(tag: String) -> Self {
        WalEntry::CancelTag { tag, ts: Utc::now() }
    }

    pub fn update(id: String, fields: HashMap<String, serde_json::Value>) -> Self {
        WalEntry::Update {
            id,
            fields,
            ts: Utc::now(),
        }
    }

    pub fn fire(id: String, fire_count: u32) -> Self {
        WalEntry::Fire {
            id,
            fire_count,
            ts: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_at: DateTime<Utc>,
    pub timers: Vec<Timer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    #[serde(flatten)]
    pub event: FireEvent,
    pub delivery_error: String,
    pub attempts: u32,
    pub outboxed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NeuroStats {
    pub uptime_secs: u64,
    pub active_timers: usize,
    pub total_created: u64,
    pub total_fired: u64,
    pub total_cancelled: u64,
    pub fires_last_hour: u64,
    pub fires_last_minute: u64,
    pub webhook_ok: u64,
    pub webhook_failed: u64,
    pub outbox_pending: usize,
    pub wal_size_bytes: u64,
    pub memory_usage_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerUpdate {
    pub payload_merge: Option<serde_json::Value>,
    pub payload_replace: Option<serde_json::Value>,
    pub priority: Option<u8>,
    pub fire_in_ms: Option<u64>,
    pub repeat_ms: Option<u64>,
    pub tags_add: Option<Vec<String>>,
    pub tags_remove: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    FireAt,
    Priority,
    CreatedAt,
}

impl std::str::FromStr for SortBy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fire_at" => Ok(SortBy::FireAt),
            "priority" => Ok(SortBy::Priority),
            "created_at" => Ok(SortBy::CreatedAt),
            _ => Err(format!("invalid sort: {}", s)),
        }
    }
}
