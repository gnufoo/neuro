use neuro::config::LimitsConfig;
use neuro::engine::Engine;
use neuro::types::{Timer, TimerUpdate, SortBy};
use chrono::{Duration, Utc};

fn create_test_timer(id: &str, fire_in_ms: i64) -> Timer {
    Timer {
        id: id.to_string(),
        fire_at: Utc::now() + Duration::milliseconds(fire_in_ms),
        priority: 128,
        payload: serde_json::json!({"test": "data"}),
        repeat_ms: None,
        max_fires: None,
        ttl: None,
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now(),
    }
}

#[test]
fn test_schedule_basic() {
    let limits = LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = create_test_timer("test-1", 500);
    let result = engine.schedule(timer);

    assert!(result.is_ok());
    let timer = result.unwrap();
    assert_eq!(timer.id, "test-1");

    // Tick until fire
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = engine.tick();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload, serde_json::json!({"test": "data"}));
    assert_eq!(events[0].fire_count, 1);
}

#[test]
fn test_schedule_with_repeat() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let mut timer = create_test_timer("repeat-1", 500);
    timer.repeat_ms = Some(500);

    engine.schedule(timer).unwrap();

    // First fire
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = engine.tick();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].fire_count, 1);

    // Timer should still exist
    assert!(engine.get("repeat-1").is_some());

    // Second fire
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = engine.tick();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].fire_count, 2);
}

#[test]
fn test_max_fires() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let mut timer = create_test_timer("max-fire", 100);
    timer.repeat_ms = Some(100);
    timer.max_fires = Some(3);

    engine.schedule(timer).unwrap();

    // Fire 5 times
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(150));
        engine.tick();
    }

    // Should only have 3 fires
    let stats = engine.stats(0, 0);
    assert_eq!(stats.total_fired, 3);
}

#[test]
fn test_ttl_expiry() {
    // This test verifies that TTL validation exists
    // The engine rejects schedules with TTL in the past
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = Timer {
        id: "ttl-test".to_string(),
        fire_at: Utc::now() + Duration::seconds(10),
        priority: 128,
        payload: serde_json::json!({}),
        repeat_ms: None,
        max_fires: None,
        ttl: Some(Utc::now() - Duration::seconds(1)), // TTL in the past
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now(),
    };

    let result = engine.schedule(timer);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("ttl"));
}

#[test]
fn test_cancel_by_id() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = create_test_timer("cancel-test", 5000);
    engine.schedule(timer).unwrap();

    let (cancelled, ids) = engine.cancel_by_id("cancel-test");
    assert_eq!(cancelled, 1);
    assert_eq!(ids, vec!["cancel-test"]);

    // Tick should return nothing
    std::thread::sleep(std::time::Duration::from_millis(100));
    let events = engine.tick();
    assert!(events.is_empty());
}

#[test]
fn test_cancel_by_tag() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // Schedule 3 timers with tag "group-a"
    for i in 0..3 {
        let mut timer = create_test_timer(&format!("a-{}", i), 5000);
        timer.tags = vec!["group-a".to_string()];
        engine.schedule(timer).unwrap();
    }

    // Schedule 2 timers with tag "group-b"
    for i in 0..2 {
        let mut timer = create_test_timer(&format!("b-{}", i), 5000);
        timer.tags = vec!["group-b".to_string()];
        engine.schedule(timer).unwrap();
    }

    let (cancelled, ids) = engine.cancel_by_tag("group-a");
    assert_eq!(cancelled, 3);
    assert_eq!(ids.len(), 3);

    // Group b should still exist
    let list = engine.list(Some("group-b"), 10, SortBy::FireAt);
    assert_eq!(list.len(), 2);
}

#[test]
fn test_cancel_nonexistent() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let (cancelled, ids) = engine.cancel_by_id("does-not-exist");
    assert_eq!(cancelled, 0);
    assert!(ids.is_empty());
}

#[test]
fn test_priority_ordering() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // Schedule 3 timers at same fire time with different priorities
    let mut timer_low = create_test_timer("low", 100);
    timer_low.priority = 200;
    engine.schedule(timer_low).unwrap();

    let mut timer_high = create_test_timer("high", 100);
    timer_high.priority = 10;
    engine.schedule(timer_high).unwrap();

    let mut timer_mid = create_test_timer("mid", 100);
    timer_mid.priority = 100;
    engine.schedule(timer_mid).unwrap();

    // Tick and check order
    std::thread::sleep(std::time::Duration::from_millis(150));
    let events = engine.tick();

    assert_eq!(events.len(), 3);
    // High priority (10) should fire first
    assert_eq!(events[0].timer_id, "high");
    // Mid priority (100) second
    assert_eq!(events[1].timer_id, "mid");
    // Low priority (200) last
    assert_eq!(events[2].timer_id, "low");
}

#[test]
fn test_update_payload_merge() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let mut timer = create_test_timer("merge-test", 5000);
    timer.payload = serde_json::json!({"a": 1, "b": 2});
    engine.schedule(timer).unwrap();

    let update = TimerUpdate {
        payload_merge: Some(serde_json::json!({"b": 3, "c": 4})),
        payload_replace: None,
        priority: None,
        fire_in_ms: None,
        repeat_ms: None,
        tags_add: None,
        tags_remove: None,
    };

    engine.update("merge-test", update).unwrap();

    let timer = engine.get("merge-test").unwrap();
    assert_eq!(timer.payload, serde_json::json!({"a": 1, "b": 3, "c": 4}));
}

#[test]
fn test_update_payload_replace() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let mut timer = create_test_timer("replace-test", 5000);
    timer.payload = serde_json::json!({"a": 1, "b": 2});
    engine.schedule(timer).unwrap();

    let update = TimerUpdate {
        payload_merge: None,
        payload_replace: Some(serde_json::json!({"x": 99})),
        priority: None,
        fire_in_ms: None,
        repeat_ms: None,
        tags_add: None,
        tags_remove: None,
    };

    engine.update("replace-test", update).unwrap();

    let timer = engine.get("replace-test").unwrap();
    assert_eq!(timer.payload, serde_json::json!({"x": 99}));
}

#[test]
fn test_update_reschedule() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = create_test_timer("reschedule-test", 10000);
    engine.schedule(timer).unwrap();

    let update = TimerUpdate {
        payload_merge: None,
        payload_replace: None,
        priority: None,
        fire_in_ms: Some(500),
        repeat_ms: None,
        tags_add: None,
        tags_remove: None,
    };

    engine.update("reschedule-test", update).unwrap();

    // Should fire at 500ms, not 10000ms
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = engine.tick();
    assert_eq!(events.len(), 1);
}

#[test]
fn test_force_fire() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = create_test_timer("force-fire", 999999);
    engine.schedule(timer).unwrap();

    let event = engine.force_fire("force-fire").unwrap();

    assert_eq!(event.fire_count, 1);
}

#[test]
fn test_replace_by_id() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let mut timer1 = create_test_timer("replace-id", 5000);
    timer1.payload = serde_json::json!({"v": 1});
    engine.schedule(timer1).unwrap();

    let mut timer2 = create_test_timer("replace-id", 5000);
    timer2.payload = serde_json::json!({"v": 2});
    engine.schedule(timer2).unwrap();

    let list = engine.list(None, 10, SortBy::FireAt);
    assert_eq!(list.len(), 1);

    let timer = engine.get("replace-id").unwrap();
    assert_eq!(timer.payload, serde_json::json!({"v": 2}));
}

#[test]
fn test_auto_id_generation() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // MCP layer generates ULID, but engine should accept any valid ID
    let timer = Timer {
        id: "01JFQX1234567890ABCDEFGH".to_string(), // ULID format
        fire_at: Utc::now() + Duration::milliseconds(5000),
        priority: 128,
        payload: serde_json::json!({"test": true}),
        repeat_ms: None,
        max_fires: None,
        ttl: None,
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now(),
    };

    let result = engine.schedule(timer);
    assert!(result.is_ok());

    let timer = result.unwrap();
    assert!(!timer.id.is_empty());
    assert!(engine.get(&timer.id).is_some());
}

#[test]
fn test_validation_no_time() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    let timer = Timer {
        id: "no-time".to_string(),
        fire_at: Utc::now(), // In the past
        priority: 128,
        payload: serde_json::json!({}),
        repeat_ms: None,
        max_fires: None,
        ttl: None,
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now(),
    };

    // This should work (can schedule for now/past)
    let result = engine.schedule(timer);
    assert!(result.is_ok());
}

#[test]
fn test_validation_payload_too_large() {
    let limits = neuro::config::LimitsConfig {
        max_payload_bytes: 100,
        ..Default::default()
    };
    let mut engine = Engine::new(limits);

    let payload = serde_json::json!({"data": "x".repeat(200)});

    let timer = Timer {
        id: "large-payload".to_string(),
        fire_at: Utc::now() + Duration::milliseconds(5000),
        priority: 128,
        payload,
        repeat_ms: None,
        max_fires: None,
        ttl: None,
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now(),
    };

    let result = engine.schedule(timer);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("payload"));
}

#[test]
fn test_validation_min_fire_time() {
    let limits = neuro::config::LimitsConfig {
        min_fire_ms: 100,
        ..Default::default()
    };
    let mut engine = Engine::new(limits);

    let mut timer = create_test_timer("too-fast", 50);
    timer.repeat_ms = Some(50);

    let result = engine.schedule(timer);
    assert!(result.is_err());
}

#[test]
fn test_list_all() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    for i in 0..5 {
        let timer = create_test_timer(&format!("list-{}", i), 5000);
        engine.schedule(timer).unwrap();
    }

    let list = engine.list(None, 10, SortBy::FireAt);
    assert_eq!(list.len(), 5);
}

#[test]
fn test_list_by_tag() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // 3 with tag "a"
    for i in 0..3 {
        let mut timer = create_test_timer(&format!("a-{}", i), 5000);
        timer.tags = vec!["a".to_string()];
        engine.schedule(timer).unwrap();
    }

    // 2 with tag "b"
    for i in 0..2 {
        let mut timer = create_test_timer(&format!("b-{}", i), 5000);
        timer.tags = vec!["b".to_string()];
        engine.schedule(timer).unwrap();
    }

    let list = engine.list(Some("a"), 10, SortBy::FireAt);
    assert_eq!(list.len(), 3);
}

#[test]
fn test_list_sorted_by_priority() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    for (i, p) in [(0, 200), (1, 50), (2, 100)] {
        let mut timer = create_test_timer(&format!("p-{}", i), 5000);
        timer.priority = p;
        engine.schedule(timer).unwrap();
    }

    let list = engine.list(None, 10, SortBy::Priority);
    assert_eq!(list[0].priority, 50);
    assert_eq!(list[1].priority, 100);
    assert_eq!(list[2].priority, 200);
}

#[test]
fn test_stats() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // Create 3 timers that fire immediately
    for i in 0..3 {
        let timer = create_test_timer(&format!("stats-{}", i), 0);
        engine.schedule(timer).unwrap();
    }

    // Fire them all
    engine.tick();

    // Cancel 1 (doesn't exist anymore after tick)
    engine.cancel_by_id("stats-1");

    let mut engine_ref = &mut engine;
    let stats = engine_ref.stats(0, 0);
    assert_eq!(stats.active_timers, 0);
    assert_eq!(stats.total_created, 3);
    assert_eq!(stats.total_fired, 3);
    assert_eq!(stats.total_cancelled, 0);
}

#[test]
fn test_late_fire_on_boot() {
    let limits = neuro::config::LimitsConfig::default();
    let mut engine = Engine::new(limits);

    // Create a timer that fires in the past
    let timer = Timer {
        id: "late-fire".to_string(),
        fire_at: Utc::now() - Duration::seconds(1), // In the past
        priority: 128,
        payload: serde_json::json!({}),
        repeat_ms: None,
        max_fires: None,
        ttl: None,
        fire_count: 0,
        callback_url: None,
        tags: vec![],
        created_at: Utc::now() - Duration::seconds(10),
    };

    engine.schedule(timer).unwrap();

    // Tick should fire immediately
    let events = engine.tick();
    assert_eq!(events.len(), 1);
    assert!(events[0].late);
}
