# Neuro - Programmable Timer Engine with MCP Interface

A Rust daemon that manages timers with arbitrary payloads, exposed via MCP (Model Context Protocol). Agents schedule, cancel, update, and query timers through standard MCP tool calls. When timers fire, the daemon delivers the payload via HTTP webhook.

## Features

- **MCP Interface** - 7 tools for timer management via JSON-RPC over SSE
- **Persistent State** - WAL (Write-Ahead Log) and snapshots for crash recovery
- **Webhook Delivery** - Automatic HTTP POST with retry logic and outbox fallback
- **Priority Queue** - Timers fire in priority order (lower = higher priority)
- **Repeating Timers** - Support for recurring timers with max fire count and TTL

## Installation

```bash
# Build the release binary
cargo build --release

# The binary will be at target/release/neuro
```

## Configuration

Create a `neuro.toml` file:

```toml
[daemon]
# Network listen address
listen = "127.0.0.1:3100"

# Timer tick interval in ms
tick_ms = 100

# State directory (WAL, snapshot, outbox)
state_dir = "./state"

[delivery]
# Default webhook URL for timer fire events
default_callback_url = "http://127.0.0.1:18789/hooks/neuro"

# Webhook timeout in seconds
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

# Minimum fire delay in ms
min_fire_ms = 100
```

## Usage

```bash
# Start the daemon
neuro -c neuro.toml

# With verbose logging
neuro -c neuro.toml -v

# Show version
neuro --version
```

## MCP Tools

### neuro_schedule

Create a new timer or replace existing (by id).

```json
{
  "name": "neuro_schedule",
  "arguments": {
    "id": "optional-timer-id",
    "fire_in_ms": 5000,
    "fire_at": "2026-03-01T15:00:00Z",
    "priority": 128,
    "payload": {"action": "notify", "message": "Hello"},
    "repeat_ms": 60000,
    "max_fires": 10,
    "ttl_ms": 3600000,
    "callback_url": "https://example.com/webhook",
    "tags": ["alerts", "critical"]
  }
}
```

Returns:
```json
{
  "timer_id": "01JFQX...",
  "fire_at": "2026-03-01T15:00:00Z",
  "status": "scheduled"
}
```

### neuro_cancel

Cancel timer(s) by id or tag.

```json
{
  "name": "neuro_cancel",
  "arguments": {
    "id": "timer-id"
  }
}
```

or

```json
{
  "name": "neuro_cancel",
  "arguments": {
    "tag": "alerts"
  }
}
```

### neuro_list

List active timers.

```json
{
  "name": "neuro_list",
  "arguments": {
    "tag": "alerts",
    "limit": 50,
    "sort": "fire_at"
  }
}
```

### neuro_get

Get full state of one timer.

```json
{
  "name": "neuro_get",
  "arguments": {
    "id": "timer-id"
  }
}
```

### neuro_update

Update a timer in-place.

```json
{
  "name": "neuro_update",
  "arguments": {
    "id": "timer-id",
    "payload_merge": {"key": "value"},
    "payload_replace": {"new": "payload"},
    "priority": 64,
    "fire_in_ms": 1000,
    "repeat_ms": 5000,
    "tags_add": ["new-tag"],
    "tags_remove": ["old-tag"]
  }
}
```

### neuro_fire

Force-fire a timer immediately.

```json
{
  "name": "neuro_fire",
  "arguments": {
    "id": "timer-id"
  }
}
```

### neuro_stats

Get daemon metrics.

```json
{
  "name": "neuro_stats"
}
```

Returns:
```json
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
```

## Webhook Payload

When a timer fires, a POST request is made to the callback URL:

```json
{
  "timer_id": "01JFQX...",
  "fired_at": "2026-03-01T15:00:00Z",
  "fire_count": 1,
  "priority": 128,
  "payload": {"action": "notify"},
  "tags": ["alerts"],
  "will_repeat": true,
  "next_fire_at": "2026-03-01T15:01:00Z",
  "late": false
}
```

## Running as a Systemd Service

1. Copy the binary to a system location:
   ```bash
   sudo cp target/release/neuro /usr/local/bin/neuro
   ```

2. Copy the config:
   ```bash
   sudo cp neuro.toml /etc/neuro/neuro.toml
   ```

3. Create the systemd service file:
   ```bash
   sudo cp neuro.service /etc/systemd/system/
   ```

4. Reload and start:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable neuro
   sudo systemctl start neuro
   ```

## Protocol

Neuro uses MCP (Model Context Protocol) over SSE transport:

- **Initialize**: `{"jsonrpc":"2.0","method":"initialize","params":{}}`
- **List Tools**: `{"jsonrpc":"2.0","method":"tools/list","params":{}}`
- **Call Tool**: `{"jsonrpc":"2.0","method":"tools/call","params":{"name":"...","arguments":{}}}`

## License

MIT
