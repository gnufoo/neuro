Neuro Phase 1 — PRD
Programmable Timer Engine with MCP Interface

Overview
Build a Rust daemon that manages timers with arbitrary payloads, exposed via MCP (Model Context Protocol). Agents schedule, cancel, update, and query timers through standard MCP tool calls. When timers fire, the daemon delivers the payload via HTTP webhook.

This is Phase 1: the core engine. No memory system integration, no wake/sleep protocols, no OpenClaw hooks. Just a rock-solid timer daemon with an MCP interface that agents can talk to.

What We're Building
A single Rust binary (neuro) that:

Runs as a localhost daemon on port 3100
Exposes 7 MCP tools via SSE transport
Manages thousands of concurrent timers with in-memory payloads
Fires timers by POSTing payloads to a webhook URL
Persists state to disk (survives crashes and restarts)
What We're NOT Building (Phase 1)
No OpenClaw integration (that's Phase 2)
No wake/sleep protocols (that's Phase 3)
No file-based memory migration (that's Phase 3)
No authentication (localhost only)
No multi-node (single machine)
No WebSocket event stream (webhook only)
Technical Requirements
Runtime
Language: Rust (2021 edition, stable toolchain)
Async runtime: Tokio
Target: Linux x86_64 (our GCP VMs run Ubuntu)
Minimum Rust version: 1.75+
Dependencies (recommended, not mandatory)
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
axum = "0.8"
reqwest = { version = "0.12", features = ["json"] }
chrono = { version = "0.4", features = ["serde"] }
ulid = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
For the timing wheel, evaluate:

hierarchical_hash_wheel_timer crate (60K downloads, 4-level hash wheel)
Or implement a simple BinaryHeap<TimerEntry> if the crate adds unnecessary complexity
For MCP:

Evaluate rmcp crate for MCP server implementation
If too heavy, implement the SSE transport manually over axum (the MCP SSE spec is simple: SSE endpoint for server→client, POST endpoint for client→server)
Daemon Behavior
Starts on 127.0.0.1:3100 (configurable)
Single-process, single-threaded timer tick loop (async)
Timer resolution: 100ms tick interval
Graceful shutdown on SIGTERM/SIGINT: flush WAL, snapshot, exit
Log to stdout (systemd captures to journal)
Log level configurable via RUST_LOG env var
Data Model
Timer
pub struct Timer {
    pub id: String,                    // ULID, auto-generated or user-provided
    pub fire_at: DateTime<Utc>,        // When to fire next
    pub priority: u8,                  // 0=highest, 255=lowest. Default: 128
    pub payload: serde_json::Value,    // Arbitrary JSON
    pub repeat_ms: Option<u64>,        // Re-enqueue interval. None = one-shot
    pub max_fires: Option<u32>,        // Auto-cancel after N fires. None = unlimited
    pub ttl: Option<DateTime<Utc>>,    // Hard expiry time. None = no expiry
    pub fire_count: u32,               // Times fired so far
    pub callback_url: Option<String>,  // Override default webhook
    pub tags: Vec<String>,             // For bulk operations
    pub created_at: DateTime<Utc>,     // When scheduled
}
FireEvent (webhook payload)
pub struct FireEvent {
    pub timer_id: String,
    pub fired_at: DateTime<Utc>,
    pub fire_count: u32,
    pub priority: u8,
    pub payload: serde_json::Value,
    pub tags: Vec<String>,
    pub will_repeat: bool,
    pub next_fire_at: Option<DateTime<Utc>>,
}
MCP Tools
The daemon exposes exactly 7 tools. Each tool has a name, description, and JSON Schema for its parameters.

1. neuro_schedule
Create a new timer or replace existing (by id).

Parameters:

{
  "type": "object",
  "properties": {
    "id":           { "type": "string", "description": "Timer ID. Auto-generated if omitted. If exists, replaces." },
    "fire_in_ms":   { "type": "number", "description": "Milliseconds from now until fire. Required if fire_at not set." },
    "fire_at":      { "type": "string", "description": "ISO 8601 absolute fire time. Required if fire_in_ms not set." },
    "priority":     { "type": "number", "description": "0-255. Default 128.", "minimum": 0, "maximum": 255 },
    "payload":      { "type": "object", "description": "Arbitrary JSON data carried with the timer." },
    "repeat_ms":    { "type": "number", "description": "Re-enqueue interval in ms after fire. Omit for one-shot." },
    "max_fires":    { "type": "number", "description": "Auto-cancel after N fires." },
    "ttl_ms":       { "type": "number", "description": "Hard expiry in ms from now." },
    "callback_url": { "type": "string", "description": "Webhook URL. Default: daemon config." },
    "tags":         { "type": "array", "items": { "type": "string" }, "description": "Tags for grouping." }
  },
  "required": []
}
Validation:

One of fire_in_ms or fire_at must be provided
fire_in_ms must be >= 100 (configurable minimum)
payload must be <= 64KB when serialized
tags max 20 per timer
Total active timers must not exceed 100,000
Returns:

{ "timer_id": "01JFQX...", "fire_at": "2026-03-01T15:00:00Z", "status": "scheduled" }
Error cases:

Missing both fire_in_ms and fire_at → error
fire_in_ms below minimum → error
Payload too large → error
Active timer limit reached → error
2. neuro_cancel
Cancel timer(s) by id or tag.

Parameters:

{
  "type": "object",
  "properties": {
    "id":  { "type": "string", "description": "Cancel specific timer." },
    "tag": { "type": "string", "description": "Cancel all timers with this tag." }
  }
}
At least one of id or tag required.

Returns:

{ "cancelled": 3, "timer_ids": ["a", "b", "c"] }
Cancelling a non-existent id returns { "cancelled": 0 } (not an error).

3. neuro_list
List active timers.

Parameters:

{
  "type": "object",
  "properties": {
    "tag":   { "type": "string", "description": "Filter by tag." },
    "limit": { "type": "number", "description": "Max results. Default 50, max 200." },
    "sort":  { "type": "string", "enum": ["fire_at", "priority", "created_at"], "description": "Sort order. Default fire_at." }
  }
}
Returns:

{
  "count": 12,
  "timers": [
    {
      "id": "moltbook-poll",
      "fire_at": "2026-03-01T14:35:00Z",
      "priority": 200,
      "fire_count": 4,
      "repeat_ms": 300000,
      "tags": ["moltbook"],
      "payload_keys": ["action", "last_karma"]
    }
  ]
}
payload_keys returns only the top-level keys (not values) to keep listing lightweight.

4. neuro_get
Get full state of one timer.

Parameters:

{
  "type": "object",
  "properties": {
    "id": { "type": "string", "description": "Timer ID." }
  },
  "required": ["id"]
}
Returns: Full Timer object as JSON (all fields including complete payload).

Error: Non-existent id → error with message "timer not found".

5. neuro_update
Update a timer in-place.

Parameters:

{
  "type": "object",
  "properties": {
    "id":              { "type": "string", "description": "Timer ID." },
    "payload_merge":   { "type": "object", "description": "Shallow merge into existing payload." },
    "payload_replace": { "type": "object", "description": "Replace entire payload." },
    "priority":        { "type": "number" },
    "fire_in_ms":      { "type": "number", "description": "Reschedule from now." },
    "repeat_ms":       { "type": "number" },
    "tags_add":        { "type": "array", "items": { "type": "string" } },
    "tags_remove":     { "type": "array", "items": { "type": "string" } }
  },
  "required": ["id"]
}
If both payload_merge and payload_replace are provided, payload_replace wins.

payload_merge does a shallow merge: top-level keys are overwritten, nested objects are not deep-merged.

Returns:

{ "timer_id": "...", "updated": ["payload", "priority"], "fire_at": "..." }
Error: Non-existent id → error.

6. neuro_fire
Force-fire a timer immediately. Timer follows normal lifecycle after (repeats if configured).

Parameters:

{
  "type": "object",
  "properties": {
    "id": { "type": "string" }
  },
  "required": ["id"]
}
Returns:

{ "fired": true, "fire_count": 5, "will_repeat": true }
The webhook is called synchronously (within the tool call). The tool returns after webhook delivery completes (or fails).

7. neuro_stats
Daemon metrics.

Parameters: none (empty object)

Returns:

{
  "uptime_secs": 86400,
  "active_timers": 12,
  "total_created": 847,
  "total_fired": 835,
  "total_cancelled": 42,
  "fires_last_hour": 23,
  "fires_last_minute": 0,
  "webhook_ok": 830,
  "webhook_failed": 5,
  "outbox_pending": 2,
  "wal_size_bytes": 4096,
  "memory_usage_bytes": 2097152
}
Webhook Delivery
When a timer fires:

Build FireEvent JSON
POST to callback_url (or default from config)
Timeout: 10 seconds
On HTTP 2xx: success, log
On HTTP 5xx or timeout or connection error:
Retry up to 3 times
Backoff: 1s, 5s, 30s
After 3 failures: write to outbox file
On HTTP 4xx: no retry, write to outbox file, log warning
Outbox File
Location: {state_dir}/outbox.jsonl

Format: one JSON object per line, same as FireEvent plus delivery metadata:

{"timer_id":"...","fired_at":"...","payload":{...},"delivery_error":"connection refused","attempts":3}
The outbox is append-only. It is NOT automatically retried. It exists for manual recovery or a future process that reads it.

Persistence
Write-Ahead Log (WAL)
Location: {state_dir}/neuro.wal

Every state-changing operation is appended to the WAL before being applied in-memory.

Format: newline-delimited JSON

{"op":"schedule","timer":{"id":"...","fire_at":"...","payload":{...},...},"ts":"2026-03-01T14:00:00Z"}
{"op":"cancel","id":"moltbook-poll","ts":"2026-03-01T14:05:00Z"}
{"op":"update","id":"nova-watch","fields":{"payload":{"last_check":"..."}},"ts":"2026-03-01T14:10:00Z"}
{"op":"fire","id":"moltbook-poll","fire_count":5,"ts":"2026-03-01T14:15:00Z"}
Snapshot
Location: {state_dir}/neuro.snap

Trigger: every 1000 WAL entries OR every 5 minutes (whichever comes first).

Format: JSON object containing all active timers.

{
  "snapshot_at": "2026-03-01T14:00:00Z",
  "timers": [
    {"id":"...","fire_at":"...","payload":{...},...},
    {"id":"...","fire_at":"...","payload":{...},...}
  ]
}
Boot Sequence
1. If neuro.snap exists:
   a. Load all timers from snapshot
   b. Replay WAL entries with ts > snapshot_at
2. Else if neuro.wal exists:
   a. Replay all WAL entries from beginning
3. Else:
   a. Start with empty state

4. Rebuild timing wheel from loaded timers
5. Truncate WAL (start fresh)
6. Write new snapshot

7. For timers whose fire_at is in the past:
   Fire immediately with "late": true in FireEvent
Configuration
File: neuro.toml (path specified via --config CLI arg)

[daemon]
# Network
listen = "127.0.0.1:3100"

# Timer engine
tick_ms = 100

# State directory (WAL, snapshot, outbox)
state_dir = "./state"

[delivery]
# Default webhook URL for timer fire events
default_callback_url = "http://127.0.0.1:18789/hooks/neuro"

# Webhook timeout
timeout_secs = 10

# Retry policy
retry_attempts = 3
retry_backoff_ms = [1000, 5000, 30000]

# Outbox file (relative to state_dir)
outbox_file = "outbox.jsonl"

[persistence]
# Snapshot triggers
snapshot_every_ops = 1000
snapshot_every_secs = 300

[limits]
# Maximum active timers
max_timers = 100000

# Maximum payload size in bytes
max_payload_bytes = 65536

# Maximum tags per timer
max_tags = 20

# Minimum fire delay
min_fire_ms = 100
CLI
neuro - Programmable timer engine with MCP interface

USAGE:
    neuro [OPTIONS]

OPTIONS:
    -c, --config <PATH>     Config file path [default: neuro.toml]
    -v, --verbose           Increase log verbosity (repeat for more: -vv, -vvv)
    --version               Print version
    --help                  Print help

EXAMPLES:
    neuro --config /etc/neuro/neuro.toml
    RUST_LOG=debug neuro -c neuro.toml
Project Structure
neuro/
├── Cargo.toml
├── neuro.toml                    # Default config (ship with binary)
├── src/
│   ├── main.rs                   # CLI parsing, daemon startup, signal handling
│   ├── config.rs                 # Config struct + TOML parsing
│   ├── types.rs                  # Timer, FireEvent, WalEntry, stats
│   ├── engine.rs                 # Timer engine: schedule, cancel, update, tick, fire
│   ├── mcp.rs                    # MCP SSE server: tool definitions, request routing
│   ├── delivery.rs               # Webhook POST with retry + outbox fallback
│   └── persistence.rs            # WAL append, snapshot write/load, boot recovery
├── tests/
│   ├── engine_test.rs
│   ├── mcp_test.rs
│   ├── delivery_test.rs
│   ├── persistence_test.rs
│   └── e2e_test.rs
└── README.md
File Responsibilities
main.rs

Parse CLI args with clap
Load config
Initialize tracing/logging
Create engine, MCP server, delivery, persistence
Start tick loop (tokio interval)
Handle SIGTERM/SIGINT for graceful shutdown
On shutdown: flush WAL, write snapshot
config.rs

Config struct with serde Deserialize
Load from TOML file
Validate (listen address, limits are sane, etc.)
Provide defaults for optional fields
types.rs

Timer struct
FireEvent struct
WalEntry enum (Schedule, Cancel, Update, Fire)
NeuroStats struct
Serialization/deserialization for all types
engine.rs

Engine struct holding: HashMap<String, Timer>, timing data structure, stats counters
fn schedule(&mut self, timer: Timer) -> Result<TimerId>
fn cancel_by_id(&mut self, id: &str) -> Result<Vec<String>>
fn cancel_by_tag(&mut self, tag: &str) -> Result<Vec<String>>
fn update(&mut self, id: &str, updates: TimerUpdate) -> Result<()>
fn tick(&mut self) -> Vec<FireEvent> — advance time, collect expired timers, sort by priority, handle repeat/max_fires/ttl
fn get(&self, id: &str) -> Option<&Timer>
fn list(&self, tag: Option<&str>, limit: usize, sort: SortBy) -> Vec<TimerSummary>
fn force_fire(&mut self, id: &str) -> Result<FireEvent>
fn stats(&self) -> NeuroStats
fn load_timers(&mut self, timers: Vec<Timer>) — for boot recovery
mcp.rs

Axum router with SSE endpoint and POST endpoint
MCP protocol: handle initialize, tools/list, tools/call
Route tool calls to engine methods
Serialize results back as MCP tool responses
Tool definitions with JSON Schema for each parameter set
delivery.rs

async fn deliver(event: &FireEvent, url: &str, config: &DeliveryConfig) -> Result<()>
Retry loop with exponential backoff
On final failure: append to outbox
fn append_outbox(event: &FireEvent, error: &str, path: &Path)
persistence.rs

fn append_wal(entry: &WalEntry, path: &Path) -> Result<()>
fn write_snapshot(timers: &[Timer], path: &Path) -> Result<()>
fn load_snapshot(path: &Path) -> Result<(DateTime<Utc>, Vec<Timer>)>
fn replay_wal(path: &Path, after: DateTime<Utc>) -> Result<Vec<WalEntry>>
fn truncate_wal(path: &Path) -> Result<()>
Boot orchestration: load snapshot, replay WAL, return timers
Test Specifications
Engine Tests (engine_test.rs)
#[test] fn test_schedule_basic()
// Schedule a timer with fire_in_ms=500.
// Call tick() until fire_at is reached.
// Assert: exactly one FireEvent returned.
// Assert: FireEvent.payload matches input payload.
// Assert: FireEvent.fire_count == 1.
// Assert: timer no longer in engine (one-shot).

#[test] fn test_schedule_with_repeat()
// Schedule timer with fire_in_ms=500, repeat_ms=500.
// Tick past first fire.
// Assert: FireEvent returned, fire_count=1.
// Assert: timer still in engine with updated fire_at.
// Tick past second fire.
// Assert: FireEvent returned, fire_count=2.

#[test] fn test_max_fires()
// Schedule with repeat_ms=100, max_fires=3.
// Tick through 5 intervals.
// Assert: exactly 3 FireEvents total.
// Assert: timer removed after 3rd fire.

#[test] fn test_ttl_expiry()
// Schedule with repeat_ms=100, ttl_ms=350.
// Assert: fires at 100, 200, 300.
// Assert: does NOT fire at 400 (TTL expired).
// Assert: timer removed.

#[test] fn test_cancel_by_id()
// Schedule timer with id="test-1".
// Cancel by id="test-1".
// Assert: cancel returns cancelled=1.
// Tick past fire_at.
// Assert: no FireEvent.

#[test] fn test_cancel_by_tag()
// Schedule 3 timers tagged "group-a", 2 tagged "group-b".
// Cancel by tag="group-a".
// Assert: cancelled=3, group-b timers still active.

#[test] fn test_cancel_nonexistent()
// Cancel id="does-not-exist".
// Assert: cancelled=0, no error.

#[test] fn test_priority_ordering()
// Schedule 3 timers at same fire_at:
//   id="low" priority=200, id="high" priority=10, id="mid" priority=100.
// Tick to fire_at.
// Assert: FireEvents returned in order: high, mid, low.

#[test] fn test_update_payload_merge()
// Schedule with payload={"a":1, "b":2}.
// Update with payload_merge={"b":3, "c":4}.
// Get timer.
// Assert: payload == {"a":1, "b":3, "c":4}.

#[test] fn test_update_payload_replace()
// Schedule with payload={"a":1, "b":2}.
// Update with payload_replace={"x":99}.
// Assert: payload == {"x":99}. Old keys gone.

#[test] fn test_update_reschedule()
// Schedule with fire_in_ms=10000 (10s).
// Update with fire_in_ms=500.
// Tick to 500ms.
// Assert: timer fires at new time, not original.

#[test] fn test_force_fire()
// Schedule with fire_in_ms=999999 (far future).
// Call force_fire.
// Assert: FireEvent returned immediately.
// Assert: if repeat, timer re-enqueued at now + repeat_ms.

#[test] fn test_replace_by_id()
// Schedule id="x" with payload={"v":1}.
// Schedule id="x" with payload={"v":2}.
// Assert: only one timer with id="x".
// Assert: payload is {"v":2}.

#[test] fn test_auto_id_generation()
// Schedule without id.
// Assert: returned timer_id is a valid ULID.
// Assert: timer exists in engine.

#[test] fn test_validation_no_time()
// Schedule without fire_in_ms or fire_at.
// Assert: error returned.

#[test] fn test_validation_payload_too_large()
// Schedule with 100KB payload.
// Assert: error returned.

#[test] fn test_validation_min_fire_time()
// Schedule with fire_in_ms=10 (below 100ms minimum).
// Assert: error returned.

#[test] fn test_list_all()
// Schedule 5 timers.
// List with no filter.
// Assert: 5 results.

#[test] fn test_list_by_tag()
// Schedule 3 tagged "a", 2 tagged "b".
// List with tag="a".
// Assert: 3 results.

#[test] fn test_list_sorted_by_priority()
// Schedule 3 timers with different priorities.
// List with sort="priority".
// Assert: ordered by priority ascending.

#[test] fn test_stats()
// Schedule 3, fire 1, cancel 1.
// Assert: active=1, total_created=3, total_fired=1, total_cancelled=1.

#[test] fn test_late_fire_on_boot()
// Create engine with a timer whose fire_at is in the past.
// Tick once.
// Assert: fires immediately.
// Assert: FireEvent includes late=true (or equivalent).
MCP Tests (mcp_test.rs)
// These tests start the full MCP server and communicate via HTTP.
// Use reqwest to send MCP messages to the SSE/POST endpoints.

#[tokio::test] async fn test_mcp_initialize()
// Send MCP initialize request.
// Assert: response includes server info and tool list.
// Assert: 7 tools listed.

#[tokio::test] async fn test_mcp_tools_list()
// Send tools/list request.
// Assert: all 7 tools present with correct schemas.

#[tokio::test] async fn test_mcp_schedule_and_list()
// Call neuro_schedule via MCP tools/call.
// Call neuro_list via MCP tools/call.
// Assert: scheduled timer appears in list.

#[tokio::test] async fn test_mcp_full_lifecycle()
// Schedule → get → update → fire → cancel
// Assert: each step returns expected result.
// Assert: final list is empty.

#[tokio::test] async fn test_mcp_error_handling()
// Call neuro_get with nonexistent id.
// Assert: MCP error response (not crash, not panic).
// Assert: error message is human-readable.

#[tokio::test] async fn test_mcp_concurrent_calls()
// Send 100 schedule calls concurrently.
// Assert: all succeed, 100 timers in list.
Delivery Tests (delivery_test.rs)
// These tests use a mock HTTP server (axum) to receive webhooks.

#[tokio::test] async fn test_webhook_success()
// Start mock server that returns 200.
// Fire a timer.
// Assert: mock received exactly one POST.
// Assert: body is valid FireEvent JSON.
// Assert: payload matches timer payload.

#[tokio::test] async fn test_webhook_retry_on_500()
// Mock returns 500 twice, then 200.
// Fire a timer.
// Assert: mock received 3 requests.
// Assert: delivered successfully on 3rd attempt.

#[tokio::test] async fn test_webhook_outbox_on_failure()
// Mock returns 500 always.
// Fire a timer.
// Assert: 3 retry attempts made (check mock call count).
// Assert: outbox.jsonl contains the fire event.
// Assert: outbox entry includes delivery_error field.

#[tokio::test] async fn test_webhook_no_retry_on_4xx()
// Mock returns 400.
// Fire a timer.
// Assert: only 1 request to mock (no retry).
// Assert: written to outbox.

#[tokio::test] async fn test_webhook_timeout()
// Mock sleeps 30s before responding.
// Fire a timer with 2s delivery timeout.
// Assert: times out, retries, eventually outbox.

#[tokio::test] async fn test_custom_callback_url()
// Schedule timer with custom callback_url pointing to mock server B.
// Default config points to mock server A.
// Fire timer.
// Assert: mock B received the request, mock A did not.
Persistence Tests (persistence_test.rs)
#[test] fn test_wal_write_and_read()
// Append 3 WAL entries (schedule, update, cancel).
// Read WAL.
// Assert: 3 entries, correct types and data.

#[test] fn test_snapshot_write_and_load()
// Create 5 timers.
// Write snapshot.
// Load snapshot.
// Assert: 5 timers recovered with correct state.

#[test] fn test_boot_from_snapshot_plus_wal()
// Write snapshot with 3 timers.
// Append WAL: schedule 2 more, cancel 1.
// Boot engine from snapshot + WAL.
// Assert: 4 timers active (3 + 2 - 1).

#[test] fn test_boot_from_wal_only()
// No snapshot file.
// WAL with 5 schedule entries.
// Boot.
// Assert: 5 timers active.

#[test] fn test_boot_empty()
// No snapshot, no WAL.
// Boot.
// Assert: 0 timers, engine running.

#[test] fn test_wal_truncation_after_snapshot()
// Append 100 WAL entries.
// Write snapshot.
// Truncate WAL.
// Assert: WAL file is empty or deleted.
// Assert: snapshot contains all state.

#[test] fn test_crash_recovery_simulation()
// Schedule 10 timers (WAL written).
// Do NOT write snapshot.
// Create new engine, boot from WAL.
// Assert: all 10 timers recovered.
// Assert: fire_at values are correct.

#[test] fn test_fire_count_persisted()
// Schedule repeating timer.
// Fire 3 times (WAL records fires).
// Restart engine.
// Assert: fire_count == 3.
End-to-End Tests (e2e_test.rs)
#[tokio::test] async fn test_e2e_schedule_wait_fire()
// Start full daemon (engine + MCP + delivery).
// Start mock webhook server.
// Call neuro_schedule via MCP: fire_in_ms=500.
// Wait 1 second.
// Assert: mock received exactly 1 webhook POST.
// Assert: FireEvent payload correct.

#[tokio::test] async fn test_e2e_repeating_timer()
// Schedule repeating timer: fire_in_ms=200, repeat_ms=200, max_fires=5.
// Wait 1.5 seconds.
// Assert: mock received exactly 5 webhooks.
// Assert: fire_count increments 1..5.
// Assert: neuro_list returns empty (timer done).

#[tokio::test] async fn test_e2e_cancel_stops_fire()
// Schedule timer fire_in_ms=1000.
// Wait 200ms.
// Cancel timer.
// Wait 1500ms.
// Assert: mock received 0 webhooks.

#[tokio::test] async fn test_e2e_update_payload_before_fire()
// Schedule timer fire_in_ms=1000, payload={"v":1}.
// Wait 200ms.
// Update payload_merge={"v":2, "extra":"yes"}.
// Wait 1200ms (timer fires).
// Assert: webhook payload == {"v":2, "extra":"yes"}.

#[tokio::test] async fn test_e2e_1000_concurrent_timers()
// Schedule 1000 timers with fire_in_ms randomly between 100-2000ms.
// Wait 3 seconds.
// Assert: mock received exactly 1000 webhooks.
// Assert: each payload unique and correct.
// Assert: neuro_stats shows total_fired=1000.

#[tokio::test] async fn test_e2e_restart_recovery()
// Start daemon.
// Schedule 5 timers, fire_in_ms=5000 each.
// Wait 1 second (timers not yet fired).
// Shutdown daemon gracefully.
// Start daemon again.
// Wait 5 seconds.
// Assert: all 5 timers fire after restart.

#[tokio::test] async fn test_e2e_priority_delivery_order()
// Schedule 3 timers at same fire_in_ms=500, priorities 200, 50, 100.
// Wait 1 second.
// Assert: mock received 3 webhooks.
// Assert: delivery order was priority 50, 100, 200.
Performance Targets
Metric	Target	How to verify
Schedule 10K timers	< 100ms	Benchmark in e2e test
Fire 1K timers in one tick	< 100ms	Benchmark in e2e test
Memory: 10K timers, 1KB payload each	< 50MB	Check process RSS
Idle CPU (100 active timers)	< 0.5%	Monitor over 1 minute
MCP tool call latency (p99)	< 10ms	Measure in MCP tests
WAL append latency (p99)	< 1ms	Measure in persistence tests
Webhook delivery latency (p99, localhost)	< 50ms	Measure in delivery tests
Deliverables
neuro binary — cargo build --release produces a single static binary
neuro.toml — default configuration file
README.md — setup, configuration, usage examples
All tests passing — cargo test with 0 failures
Benchmarks — cargo bench or inline timing in e2e tests showing performance targets met
Systemd unit file — neuro.service for daemon management
Definition of Done
Phase 1 is complete when:

[ ] cargo build --release succeeds with no warnings
[ ] cargo test passes all tests listed above (33 tests minimum)
[ ] cargo clippy has no warnings
[ ] All 7 MCP tools work correctly via the SSE transport
[ ] Timers fire within 200ms of expected time
[ ] Webhook delivery with retry and outbox fallback works
[ ] WAL + snapshot persistence survives kill -9 and restart
[ ] Performance targets met on a 2-vCPU GCP e2-standard-2 instance
[ ] README documents setup, config, and all MCP tool usage
[ ] Can run as systemd service
