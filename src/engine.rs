use crate::config::LimitsConfig;
use crate::types::{
    FireEvent, NeuroStats, SortBy, Timer, TimerSummary, TimerUpdate,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

pub struct Engine {
    timers: HashMap<String, Timer>,
    timers_by_tag: HashMap<String, Vec<String>>,
    stats: EngineStats,
    limits: LimitsConfig,
    ops_since_snapshot: usize,
    started_at: DateTime<Utc>,
    last_hour_fires: Vec<DateTime<Utc>>,
    last_minute_fires: Vec<DateTime<Utc>>,
}

#[derive(Default)]
struct EngineStats {
    total_created: u64,
    total_fired: u64,
    total_cancelled: u64,
    webhook_ok: u64,
    webhook_failed: u64,
}

impl Engine {
    pub fn new(limits: LimitsConfig) -> Self {
        Self {
            timers: HashMap::new(),
            timers_by_tag: HashMap::new(),
            stats: EngineStats::default(),
            limits,
            ops_since_snapshot: 0,
            started_at: Utc::now(),
            last_hour_fires: Vec::new(),
            last_minute_fires: Vec::new(),
        }
    }

    pub fn schedule(&mut self, timer: Timer) -> Result<Timer, String> {
        if self.timers.len() >= self.limits.max_timers {
            return Err("active timer limit reached".to_string());
        }

        let payload_bytes = serde_json::to_vec(&timer.payload)
            .map_err(|e| e.to_string())?
            .len();
        if payload_bytes > self.limits.max_payload_bytes {
            return Err("payload too large".to_string());
        }

        if timer.tags.len() > self.limits.max_tags {
            return Err("too many tags".to_string());
        }

        if let Some(repeat) = timer.repeat_ms {
            if repeat < self.limits.min_fire_ms {
                return Err("repeat interval below minimum".to_string());
            }
        }

        if timer.ttl.map_or(false, |ttl| ttl < Utc::now()) {
            return Err("ttl is in the past".to_string());
        }

        // Remove old timer if replacing
        let old_tags = if let Some(old) = self.timers.get(&timer.id) {
            old.tags.clone()
        } else {
            vec![]
        };

        for tag in old_tags {
            if let Some(ids) = self.timers_by_tag.get_mut(&tag) {
                ids.retain(|id| id != &timer.id);
                if ids.is_empty() {
                    self.timers_by_tag.remove(&tag);
                }
            }
        }

        let timer_id = timer.id.clone();
        self.add_timer_to_tags(&timer);

        self.timers.insert(timer.id.clone(), timer);
        self.stats.total_created += 1;
        self.ops_since_snapshot += 1;

        Ok(self.timers.get(&timer_id).cloned().unwrap())
    }

    fn add_timer_to_tags(&mut self, timer: &Timer) {
        for tag in &timer.tags {
            self.timers_by_tag
                .entry(tag.clone())
                .or_default()
                .push(timer.id.clone());
        }
    }

    fn remove_timer_from_tags(&mut self, timer: &Timer) {
        for tag in &timer.tags {
            if let Some(ids) = self.timers_by_tag.get_mut(tag) {
                ids.retain(|id| id != &timer.id);
                if ids.is_empty() {
                    self.timers_by_tag.remove(tag);
                }
            }
        }
    }

    pub fn cancel_by_id(&mut self, id: &str) -> (usize, Vec<String>) {
        if let Some(timer) = self.timers.remove(id) {
            self.remove_timer_from_tags(&timer);
            self.stats.total_cancelled += 1;
            self.ops_since_snapshot += 1;
            (1, vec![id.to_string()])
        } else {
            (0, vec![])
        }
    }

    pub fn cancel_by_tag(&mut self, tag: &str) -> (usize, Vec<String>) {
        let ids = self.timers_by_tag.remove(tag).unwrap_or_default();
        let count = ids.len();
        for id in &ids {
            if let Some(timer) = self.timers.remove(id) {
                self.remove_timer_from_tags(&timer);
            }
        }
        if count > 0 {
            self.stats.total_cancelled += count as u64;
            self.ops_since_snapshot += 1;
        }
        (count, ids)
    }

    pub fn update(&mut self, id: &str, update: TimerUpdate) -> Result<Option<DateTime<Utc>>, String> {
        let timer = self.timers.get_mut(id).ok_or("timer not found")?;

        if let Some(priority) = update.priority {
            timer.priority = priority;
        }

        if let Some(repeat_ms) = update.repeat_ms {
            timer.repeat_ms = Some(repeat_ms);
        }

        if let Some(fire_in_ms) = update.fire_in_ms {
            timer.fire_at = Utc::now() + Duration::milliseconds(fire_in_ms as i64);
        }

        if let Some(tags_add) = update.tags_add {
            for tag in &tags_add {
                if !timer.tags.contains(tag) {
                    if timer.tags.len() < self.limits.max_tags {
                        timer.tags.push(tag.clone());
                        self.timers_by_tag
                            .entry(tag.clone())
                            .or_default()
                            .push(id.to_string());
                    }
                }
            }
        }

        if let Some(tags_remove) = update.tags_remove {
            for tag in &tags_remove {
                timer.tags.retain(|t| t != tag);
            }
            // Rebuild tags index
            self.timers_by_tag.retain(|_, ids| {
                ids.retain(|i| i != id);
                !ids.is_empty()
            });
            for tag in &timer.tags {
                self.timers_by_tag
                    .entry(tag.clone())
                    .or_default()
                    .push(id.to_string());
            }
        }

        if let Some(replace) = update.payload_replace {
            timer.payload = replace;
        } else if let Some(merge) = update.payload_merge {
            if let Some(obj) = timer.payload.as_object_mut() {
                if let Some(merge_obj) = merge.as_object() {
                    for (k, v) in merge_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            } else {
                timer.payload = merge;
            }
        }

        self.ops_since_snapshot += 1;
        Ok(Some(timer.fire_at))
    }

    pub fn tick(&mut self) -> Vec<FireEvent> {
        let now = Utc::now();
        let mut fired_timers: Vec<(String, Timer)> = Vec::new();

        // Collect timers that should fire
        let should_fire: Vec<String> = self.timers
            .iter()
            .filter(|(_, timer)| {
                if timer.fire_at <= now {
                    if let Some(ttl) = timer.ttl {
                        if now > ttl {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            })
            .map(|(id, timer)| {
                fired_timers.push((id.clone(), timer.clone()));
                id.clone()
            })
            .collect();

        for id in &should_fire {
            self.timers.remove(id);
        }

        // Remove from tags index
        for (_, timer) in &fired_timers {
            self.remove_timer_from_tags(timer);
        }

        // Sort by priority (lower = higher priority, so ascending order)
        fired_timers.sort_by(|a, b| a.1.priority.cmp(&b.1.priority));

        let mut events = Vec::new();

        for (id, mut timer) in fired_timers {
            let late = timer.fire_at < now - Duration::milliseconds(200);

            // Record fire time for stats
            self.last_hour_fires.push(now);
            self.last_minute_fires.push(now);
            self.stats.total_fired += 1;
            self.ops_since_snapshot += 1;

            // Check if should repeat
            let should_repeat = timer.repeat_ms.map_or(false, |_| {
                timer.max_fires.map_or(true, |max| timer.fire_count + 1 < max)
            });

            timer.fire_count += 1;

            let event = FireEvent::from_timer(&timer, now, late);

            if should_repeat {
                // Re-enqueue timer
                let new_fire_at = now + Duration::milliseconds(timer.repeat_ms.unwrap() as i64);

                // Check TTL - don't re-enqueue if TTL expired (compare next fire time against TTL)
                let can_repeat = if let Some(ttl) = timer.ttl {
                    new_fire_at <= ttl
                } else {
                    true
                };

                if can_repeat {
                    timer.fire_at = new_fire_at;
                    self.timers.insert(id.clone(), timer.clone());
                    self.add_timer_to_tags(&timer);
                }
            }

            events.push(event);
        }

        events
    }

    pub fn get(&self, id: &str) -> Option<&Timer> {
        self.timers.get(id)
    }

    pub fn list(&self, tag: Option<&str>, limit: usize, sort: SortBy) -> Vec<TimerSummary> {
        let mut timers: Vec<&Timer> = if let Some(tag) = tag {
            self.timers_by_tag
                .get(tag)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| self.timers.get(id))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            self.timers.values().collect()
        };

        match sort {
            SortBy::FireAt => timers.sort_by(|a, b| a.fire_at.cmp(&b.fire_at)),
            SortBy::Priority => timers.sort_by(|a, b| a.priority.cmp(&b.priority)),
            SortBy::CreatedAt => timers.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        }

        let limit = limit.min(200);
        timers.truncate(limit);

        timers.iter().map(|t| TimerSummary::from(*t)).collect()
    }

    pub fn force_fire(&mut self, id: &str) -> Result<FireEvent, String> {
        let timer = self.timers.get_mut(id).ok_or("timer not found")?.clone();

        let now = Utc::now();
        let late = true;

        // Remove from engine
        self.timers.remove(id);
        self.remove_timer_from_tags(&timer);

        let mut timer = timer;
        timer.fire_count += 1;

        self.stats.total_fired += 1;
        self.ops_since_snapshot += 1;

        let will_repeat = timer.repeat_ms.is_some()
            && timer.max_fires.map_or(true, |max| timer.fire_count < max);

        let event = FireEvent {
            timer_id: timer.id.clone(),
            fired_at: now,
            fire_count: timer.fire_count,
            priority: timer.priority,
            payload: timer.payload.clone(),
            tags: timer.tags.clone(),
            will_repeat,
            next_fire_at: None,
            late,
        };

        // Re-enqueue if repeating
        if will_repeat {
            let new_fire_at = now + Duration::milliseconds(timer.repeat_ms.unwrap() as i64);
            timer.fire_at = new_fire_at;
            if timer.ttl.map_or(true, |ttl| new_fire_at <= ttl) {
                self.timers.insert(timer.id.clone(), timer.clone());
                self.add_timer_to_tags(&timer);
            }
        }

        Ok(event)
    }

    pub fn stats(&mut self, wal_size_bytes: u64, outbox_pending: usize) -> NeuroStats {
        let now = Utc::now();
        let uptime = (now - self.started_at).num_seconds();

        // Clean old fire times
        let one_hour_ago = now - Duration::hours(1);
        let one_minute_ago = now - Duration::minutes(1);
        self.last_hour_fires.retain(|t| *t > one_hour_ago);
        self.last_minute_fires.retain(|t| *t > one_minute_ago);

        NeuroStats {
            uptime_secs: uptime as u64,
            active_timers: self.timers.len(),
            total_created: self.stats.total_created,
            total_fired: self.stats.total_fired,
            total_cancelled: self.stats.total_cancelled,
            fires_last_hour: self.last_hour_fires.len() as u64,
            fires_last_minute: self.last_minute_fires.len() as u64,
            webhook_ok: self.stats.webhook_ok,
            webhook_failed: self.stats.webhook_failed,
            outbox_pending,
            wal_size_bytes,
            memory_usage_bytes: 0, // Would need to measure properly
        }
    }

    pub fn load_timers(&mut self, timers: Vec<Timer>) {
        for timer in timers {
            self.add_timer_to_tags(&timer);
            self.timers.insert(timer.id.clone(), timer);
        }
    }

    pub fn ops_since_snapshot(&self) -> usize {
        self.ops_since_snapshot
    }

    pub fn reset_ops_since_snapshot(&mut self) {
        self.ops_since_snapshot = 0;
    }

    pub fn record_webhook_success(&mut self) {
        self.stats.webhook_ok += 1;
    }

    pub fn record_webhook_failure(&mut self) {
        self.stats.webhook_failed += 1;
    }

    pub fn timer_count(&self) -> usize {
        self.timers.len()
    }
}
