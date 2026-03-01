use crate::types::{Snapshot, Timer, WalEntry};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn};

#[derive(Clone)]
pub struct Persistence {
    state_dir: PathBuf,
    wal_path: PathBuf,
    snap_path: PathBuf,
}

impl Persistence {
    pub fn new(state_dir: PathBuf) -> Self {
        let wal_path = state_dir.join("neuro.wal");
        let snap_path = state_dir.join("neuro.snap");

        Self {
            state_dir,
            wal_path,
            snap_path,
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn wal_path(&self) -> &Path {
        &self.wal_path
    }

    pub fn snap_path(&self) -> &Path {
        &self.snap_path
    }

    pub async fn append_wal(&self, entry: &WalEntry) -> Result<(), PersistenceError> {
        let line = serde_json::to_string(entry).map_err(PersistenceError::SerializeError)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.wal_path)
            .await
            .map_err(PersistenceError::IoError)?;

        file.write_all(line.as_bytes()).await.map_err(PersistenceError::IoError)?;
        file.write_all(b"\n").await.map_err(PersistenceError::IoError)?;

        Ok(())
    }

    pub async fn write_snapshot(&self, timers: &[Timer]) -> Result<(), PersistenceError> {
        let snapshot = Snapshot {
            snapshot_at: Utc::now(),
            timers: timers.to_vec(),
        };

        let json = serde_json::to_string_pretty(&snapshot).map_err(PersistenceError::SerializeError)?;

        let mut file = File::create(&self.snap_path).await.map_err(PersistenceError::IoError)?;

        file.write_all(json.as_bytes()).await.map_err(PersistenceError::IoError)?;

        info!("wrote snapshot with {} timers", timers.len());

        Ok(())
    }

    pub async fn load_snapshot(&self) -> Result<Option<(DateTime<Utc>, Vec<Timer>)>, PersistenceError> {
        if !self.snap_path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&self.snap_path)
            .await
            .map_err(PersistenceError::IoError)?;

        let snapshot: Snapshot = serde_json::from_str(&content).map_err(PersistenceError::DeserializeError)?;

        Ok(Some((snapshot.snapshot_at, snapshot.timers)))
    }

    pub async fn replay_wal(&self, after: Option<DateTime<Utc>>) -> Result<Vec<WalEntry>, PersistenceError> {
        if !self.wal_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.wal_path).await.map_err(PersistenceError::IoError)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let mut entries = Vec::new();

        while let Some(line) = lines.next_line().await.map_err(PersistenceError::IoError)? {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(entry) = serde_json::from_str::<WalEntry>(&line) {
                if let Some(after_ts) = after {
                    if entry.ts() > after_ts {
                        entries.push(entry);
                    }
                } else {
                    entries.push(entry);
                }
            } else {
                warn!("failed to parse WAL entry: {}", line);
            }
        }

        Ok(entries)
    }

    pub async fn truncate_wal(&self) -> Result<(), PersistenceError> {
        if self.wal_path.exists() {
            tokio::fs::remove_file(&self.wal_path)
                .await
                .map_err(PersistenceError::IoError)?;
        }
        Ok(())
    }

    pub async fn wal_size(&self) -> u64 {
        if let Ok(meta) = tokio::fs::metadata(&self.wal_path).await {
            meta.len()
        } else {
            0
        }
    }

    pub async fn boot(&self) -> Result<Vec<Timer>, PersistenceError> {
        // Try snapshot first
        if let Some((snap_ts, mut timers)) = self.load_snapshot().await? {
            info!("loaded snapshot with {} timers from {}", timers.len(), snap_ts);

            // Replay WAL entries after snapshot
            let wal_entries = self.replay_wal(Some(snap_ts)).await?;

            for entry in wal_entries {
                timers = self.apply_wal_entry(timers, entry);
            }

            // Truncate WAL after successful recovery
            self.truncate_wal().await?;

            // Write new snapshot
            self.write_snapshot(&timers).await?;

            return Ok(timers);
        }

        // No snapshot, try WAL only
        if self.wal_path.exists() {
            let wal_entries = self.replay_wal(None).await?;
            let mut timers = Vec::new();

            for entry in wal_entries {
                timers = self.apply_wal_entry(timers, entry);
            }

            self.truncate_wal().await?;
            self.write_snapshot(&timers).await?;

            return Ok(timers);
        }

        // Empty start
        Ok(Vec::new())
    }

    fn apply_wal_entry(&self, mut timers: Vec<Timer>, entry: WalEntry) -> Vec<Timer> {
        match entry {
            WalEntry::Schedule { timer, .. } => {
                timers.retain(|t| t.id != timer.id);
                timers.push(timer);
            }
            WalEntry::Cancel { id, .. } => {
                timers.retain(|t| t.id != id);
            }
            WalEntry::CancelTag { tag, .. } => {
                timers.retain(|t| !t.tags.contains(&tag));
            }
            WalEntry::Update { id, fields, .. } => {
                if let Some(timer) = timers.iter_mut().find(|t| t.id == id) {
                    if let Some(payload) = fields.get("payload") {
                        timer.payload = payload.clone();
                    }
                    if let Some(priority) = fields.get("priority").and_then(|v| v.as_u64()) {
                        timer.priority = priority as u8;
                    }
                    if let Some(fire_at) = fields.get("fire_at").and_then(|v| v.as_str()) {
                        if let Ok(dt) = fire_at.parse::<DateTime<Utc>>() {
                            timer.fire_at = dt;
                        }
                    }
                    if let Some(repeat_ms) = fields.get("repeat_ms").and_then(|v| v.as_u64()) {
                        timer.repeat_ms = Some(repeat_ms);
                    }
                }
            }
            WalEntry::Fire { id, fire_count, .. } => {
                if let Some(timer) = timers.iter_mut().find(|t| t.id == id) {
                    timer.fire_count = fire_count;
                }
            }
        }
        timers
    }
}

#[derive(Debug)]
pub enum PersistenceError {
    IoError(std::io::Error),
    SerializeError(serde_json::Error),
    DeserializeError(serde_json::Error),
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistenceError::IoError(e) => write!(f, "I/O error: {}", e),
            PersistenceError::SerializeError(e) => write!(f, "serialization error: {}", e),
            PersistenceError::DeserializeError(e) => write!(f, "deserialization error: {}", e),
        }
    }
}

impl std::error::Error for PersistenceError {}

trait WalEntryExt {
    fn ts(&self) -> DateTime<Utc>;
}

impl WalEntryExt for WalEntry {
    fn ts(&self) -> DateTime<Utc> {
        match self {
            WalEntry::Schedule { ts, .. } => *ts,
            WalEntry::Cancel { ts, .. } => *ts,
            WalEntry::CancelTag { ts, .. } => *ts,
            WalEntry::Update { ts, .. } => *ts,
            WalEntry::Fire { ts, .. } => *ts,
        }
    }
}
