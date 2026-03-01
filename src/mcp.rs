use crate::config::DeliveryConfig;
use crate::delivery::Delivery;
use crate::engine::Engine;
use crate::persistence::Persistence;
use crate::types::{
    SortBy, Timer, TimerUpdate, WalEntry,
};
use std::collections::HashMap;
use axum::{
    extract::State,
    response::{sse::{Event, Sse}, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::Mutex;
use ulid::Ulid;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<Engine>>,
    pub delivery: Arc<Delivery>,
    pub persistence: Arc<Persistence>,
    pub config: DeliveryConfig,
}

pub fn create_mcp_server(
    engine: Arc<Mutex<Engine>>,
    delivery: Delivery,
    persistence: Persistence,
    config: DeliveryConfig,
) -> Router {
    let state = AppState {
        engine,
        delivery: Arc::new(delivery),
        persistence: Arc::new(persistence),
        config,
    };

    Router::new()
        .route("/mcp", get(sse_handler))
        .route("/mcp", post(tool_handler))
        .with_state(state)
}

async fn sse_handler() -> Sse<futures::stream::Iter<std::vec::IntoIter<Result<Event, Infallible>>>> {
    let event = Event::default()
        .data(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);

    let v = vec![Ok(event)];
    Sse::new(futures::stream::iter(v))
}

#[derive(serde::Deserialize)]
struct McpRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    params: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct McpResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpError>,
}

#[derive(serde::Serialize)]
struct McpError {
    code: i32,
    message: String,
}

async fn tool_handler(
    State(state): State<AppState>,
    Json(req): Json<McpRequest>,
) -> Response {
    if req.jsonrpc != "2.0" {
        return Json(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: None,
            error: Some(McpError {
                code: -32600,
                message: "Invalid JSON-RPC".to_string(),
            }),
        })
        .into_response();
    }

    let result = match req.method.as_str() {
        "tools/list" => handle_tools_list(),
        "tools/call" => {
            let params = req.params.unwrap_or(serde_json::Value::Object(Default::default()));
            handle_tools_call(&state, params).await
        }
        "initialize" => handle_initialize(),
        _ => Err(format!("unknown method: {}", req.method)),
    };

    match result {
        Ok(result) => Json(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: Some(result),
            error: None,
        })
        .into_response(),
        Err(msg) => Json(McpResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: None,
            error: Some(McpError {
                code: -32000,
                message: msg,
            }),
        })
        .into_response(),
    }
}

fn handle_initialize() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "protocolVersion": "2024-11-05",
        "serverInfo": {
            "name": "neuro",
            "version": "0.1.0"
        },
        "capabilities": {
            "tools": {}
        }
    }))
}

fn handle_tools_list() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "tools": [
            {
                "name": "neuro_schedule",
                "description": "Create a new timer or replace existing (by id)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "Timer ID. Auto-generated if omitted."},
                        "fire_in_ms": {"type": "number", "description": "Milliseconds from now until fire."},
                        "fire_at": {"type": "string", "description": "ISO 8601 absolute fire time."},
                        "priority": {"type": "number", "minimum": 0, "maximum": 255},
                        "payload": {"type": "object"},
                        "repeat_ms": {"type": "number"},
                        "max_fires": {"type": "number"},
                        "ttl_ms": {"type": "number"},
                        "callback_url": {"type": "string"},
                        "tags": {"type": "array", "items": {"type": "string"}}
                    }
                }
            },
            {
                "name": "neuro_cancel",
                "description": "Cancel timer(s) by id or tag",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "tag": {"type": "string"}
                    }
                }
            },
            {
                "name": "neuro_list",
                "description": "List active timers",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tag": {"type": "string"},
                        "limit": {"type": "number"},
                        "sort": {"type": "string", "enum": ["fire_at", "priority", "created_at"]}
                    }
                }
            },
            {
                "name": "neuro_get",
                "description": "Get full state of one timer",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "neuro_update",
                "description": "Update a timer in-place",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "payload_merge": {"type": "object"},
                        "payload_replace": {"type": "object"},
                        "priority": {"type": "number"},
                        "fire_in_ms": {"type": "number"},
                        "repeat_ms": {"type": "number"},
                        "tags_add": {"type": "array", "items": {"type": "string"}},
                        "tags_remove": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "neuro_fire",
                "description": "Force-fire a timer immediately",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "neuro_stats",
                "description": "Daemon metrics",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    }))
}

async fn handle_tools_call(
    state: &AppState,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("missing tool name")?;

    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    match tool_name {
        "neuro_schedule" => handle_schedule(state, args).await,
        "neuro_cancel" => handle_cancel(state, args).await,
        "neuro_list" => handle_list(state, args).await,
        "neuro_get" => handle_get(state, args).await,
        "neuro_update" => handle_update(state, args).await,
        "neuro_fire" => handle_fire(state, args).await,
        "neuro_stats" => handle_stats(state).await,
        _ => Err(format!("unknown tool: {}", tool_name)),
    }
}

async fn handle_schedule(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let fire_in_ms = args.get("fire_in_ms").and_then(|v| v.as_u64());
    let fire_at_str = args.get("fire_at").and_then(|v| v.as_str());

    if fire_in_ms.is_none() && fire_at_str.is_none() {
        return Err("one of fire_in_ms or fire_at required".to_string());
    }

    let fire_at = if let Some(ms) = fire_in_ms {
        Utc::now() + Duration::milliseconds(ms as i64)
    } else {
        DateTime::parse_from_rfc3339(fire_at_str.unwrap())
            .map_err(|e| e.to_string())?
            .with_timezone(&Utc)
    };

    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| Ulid::new().to_string());

    let priority = args
        .get("priority")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8)
        .unwrap_or(128);

    let payload = args
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let repeat_ms = args.get("repeat_ms").and_then(|v| v.as_u64());
    let max_fires = args.get("max_fires").and_then(|v| v.as_u64()).map(|v| v as u32);
    let ttl_ms = args.get("ttl_ms").and_then(|v| v.as_u64());
    let callback_url = args.get("callback_url").and_then(|v| v.as_str()).map(String::from);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let ttl = ttl_ms.map(|ms| Utc::now() + Duration::milliseconds(ms as i64));

    let timer = Timer {
        id: id.clone(),
        fire_at,
        priority,
        payload,
        repeat_ms,
        max_fires,
        ttl,
        fire_count: 0,
        callback_url,
        tags,
        created_at: Utc::now(),
    };

    let mut engine = state.engine.lock().await;
    let result = engine.schedule(timer)?;

    // Write to WAL
    let wal_entry = WalEntry::schedule(result.clone());
    state.persistence.append_wal(&wal_entry).await.map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "timer_id": result.id,
        "fire_at": result.fire_at.to_rfc3339(),
        "status": "scheduled"
    }))
}

async fn handle_cancel(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = args.get("id").and_then(|v| v.as_str()).map(String::from);
    let tag = args.get("tag").and_then(|v| v.as_str()).map(String::from);

    let cancel_id = id.clone();
    let cancel_tag = tag.clone();

    let mut engine = state.engine.lock().await;
    let (cancelled, timer_ids) = if let Some(ref id) = id {
        engine.cancel_by_id(id)
    } else if let Some(ref tag) = tag {
        engine.cancel_by_tag(tag)
    } else {
        return Err("id or tag required".to_string());
    };

    // Write to WAL
    if let Some(id) = cancel_id {
        let wal_entry = WalEntry::cancel(id);
        state.persistence.append_wal(&wal_entry).await.map_err(|e| e.to_string())?;
    } else if let Some(tag) = cancel_tag {
        let wal_entry = WalEntry::cancel_tag(tag);
        state.persistence.append_wal(&wal_entry).await.map_err(|e| e.to_string())?;
    }

    Ok(serde_json::json!({
        "cancelled": cancelled,
        "timer_ids": timer_ids
    }))
}

async fn handle_list(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let tag = args.get("tag").and_then(|v| v.as_str());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(50);
    let sort_str = args
        .get("sort")
        .and_then(|v| v.as_str())
        .unwrap_or("fire_at");
    let sort = sort_str.parse::<SortBy>().unwrap_or_default();

    let engine = state.engine.lock().await;
    let timers = engine.list(tag, limit, sort);

    Ok(serde_json::json!({
        "count": timers.len(),
        "timers": timers
    }))
}

async fn handle_get(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id required")?;

    let engine = state.engine.lock().await;
    let timer = engine.get(id).ok_or("timer not found")?;

    Ok(serde_json::json!(timer))
}

async fn handle_update(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id required")?;

    let update = TimerUpdate {
        payload_merge: args.get("payload_merge").cloned(),
        payload_replace: args.get("payload_replace").cloned(),
        priority: args.get("priority").and_then(|v| v.as_u64()).map(|v| v as u8),
        fire_in_ms: args.get("fire_in_ms").and_then(|v| v.as_u64()),
        repeat_ms: args.get("repeat_ms").and_then(|v| v.as_u64()),
        tags_add: args.get("tags_add").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        }),
        tags_remove: args.get("tags_remove").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        }),
    };

    let mut engine = state.engine.lock().await;
    let fire_at = engine.update(id, update.clone())?.ok_or("timer not found")?;

    // Write to WAL with actual field values
    let mut fields = HashMap::new();
    if update.payload_merge.is_some() || update.payload_replace.is_some() {
        if let Some(timer) = engine.get(id) {
            fields.insert("payload".to_string(), timer.payload.clone());
        }
    }
    if update.priority.is_some() {
        if let Some(timer) = engine.get(id) {
            fields.insert("priority".to_string(), serde_json::json!(timer.priority));
        }
    }
    if update.fire_in_ms.is_some() || update.repeat_ms.is_some() {
        if let Some(timer) = engine.get(id) {
            fields.insert("fire_at".to_string(), serde_json::json!(timer.fire_at.to_rfc3339()));
        }
    }
    if update.repeat_ms.is_some() {
        if let Some(timer) = engine.get(id) {
            fields.insert("repeat_ms".to_string(), serde_json::json!(timer.repeat_ms));
        }
    }

    let wal_entry = WalEntry::update(id.to_string(), fields);
    state.persistence.append_wal(&wal_entry).await.map_err(|e| e.to_string())?;

    // Track what was actually updated
    let mut updated_fields = Vec::new();
    if update.payload_merge.is_some() || update.payload_replace.is_some() {
        updated_fields.push("payload");
    }
    if update.priority.is_some() {
        updated_fields.push("priority");
    }
    if update.fire_in_ms.is_some() {
        updated_fields.push("fire_at");
    }
    if update.repeat_ms.is_some() {
        updated_fields.push("repeat_ms");
    }
    if update.tags_add.is_some() || update.tags_remove.is_some() {
        updated_fields.push("tags");
    }

    Ok(serde_json::json!({
        "timer_id": id,
        "updated": updated_fields,
        "fire_at": fire_at.to_rfc3339()
    }))
}

async fn handle_fire(
    state: &AppState,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("id required")?;

    let event = {
        let mut engine = state.engine.lock().await;
        engine.force_fire(id)?
    };

    // Write to WAL
    let wal_entry = WalEntry::fire(id.to_string(), event.fire_count);
    state.persistence.append_wal(&wal_entry).await.map_err(|e| e.to_string())?;

    // Deliver webhook
    let url = state
        .delivery
        .default_url()
        .to_string();

    let outbox_path = state.delivery.outbox_path(state.persistence.state_dir());

    let result = state.delivery.deliver(&event, &url, &outbox_path).await;

    let mut engine = state.engine.lock().await;
    if result.is_ok() {
        engine.record_webhook_success();
    } else {
        engine.record_webhook_failure();
    }

    Ok(serde_json::json!({
        "fired": true,
        "fire_count": event.fire_count,
        "will_repeat": event.will_repeat
    }))
}

async fn handle_stats(state: &AppState) -> Result<serde_json::Value, String> {
    let mut engine = state.engine.lock().await;
    let wal_size = state.persistence.wal_size().await;

    // Count outbox entries
    let outbox_path = state.delivery.outbox_path(state.persistence.state_dir());
    let outbox_pending = if outbox_path.exists() {
        let content = tokio::fs::read_to_string(&outbox_path)
            .await
            .unwrap_or_default();
        content.lines().filter(|l| !l.is_empty()).count()
    } else {
        0
    };

    let stats = engine.stats(wal_size, outbox_pending);

    Ok(serde_json::json!(stats))
}
