use clap::{Parser, Subcommand};
use serde_json::json;

const DEFAULT_URL: &str = "http://127.0.0.1:3100";

#[derive(Parser, Debug)]
#[command(name = "neuro-cli")]
#[command(about = "CLI client for the Neuro timer engine")]
#[command(version)]
pub struct Cli {
    /// Daemon URL
    #[arg(long, default_value = DEFAULT_URL, global = true)]
    pub url: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Schedule a new timer
    Schedule {
        /// Timer ID (auto-generated if omitted)
        #[arg(long)]
        id: Option<String>,

        /// Fire after N milliseconds
        #[arg(long)]
        fire_in_ms: Option<u64>,

        /// Fire at ISO 8601 timestamp
        #[arg(long)]
        fire_at: Option<String>,

        /// Fire after human duration (e.g. 5m, 1h, 30s)
        #[arg(long, short = 'i')]
        r#in: Option<String>,

        /// Priority (0-255, lower = higher priority)
        #[arg(long, short, default_value = "128")]
        priority: u8,

        /// JSON payload
        #[arg(long)]
        payload: Option<String>,

        /// Repeat interval in ms
        #[arg(long)]
        repeat_ms: Option<u64>,

        /// Repeat with human duration (e.g. 30m, 1h)
        #[arg(long)]
        repeat: Option<String>,

        /// Maximum number of fires
        #[arg(long)]
        max_fires: Option<u32>,

        /// TTL in ms
        #[arg(long)]
        ttl_ms: Option<u64>,

        /// Callback URL override
        #[arg(long)]
        callback_url: Option<String>,

        /// Tags (comma-separated or repeated)
        #[arg(long, short, value_delimiter = ',')]
        tags: Vec<String>,
    },

    /// Cancel timer(s) by id or tag
    Cancel {
        /// Timer ID
        #[arg(long)]
        id: Option<String>,

        /// Cancel all timers with this tag
        #[arg(long)]
        tag: Option<String>,
    },

    /// List active timers
    #[command(alias = "ls")]
    List {
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Max results
        #[arg(long, short, default_value = "20")]
        limit: usize,

        /// Sort by: fire_at, priority, created_at
        #[arg(long, short, default_value = "fire_at")]
        sort: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Get full state of a timer
    Get {
        /// Timer ID
        id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Update a timer in-place
    Update {
        /// Timer ID
        id: String,

        /// New priority
        #[arg(long, short)]
        priority: Option<u8>,

        /// Reschedule: fire in N ms
        #[arg(long)]
        fire_in_ms: Option<u64>,

        /// Reschedule: fire in human duration
        #[arg(long, short = 'i')]
        r#in: Option<String>,

        /// New repeat interval in ms
        #[arg(long)]
        repeat_ms: Option<u64>,

        /// Merge JSON into payload
        #[arg(long)]
        payload_merge: Option<String>,

        /// Replace payload entirely
        #[arg(long)]
        payload_replace: Option<String>,

        /// Add tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags_add: Vec<String>,

        /// Remove tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags_remove: Vec<String>,
    },

    /// Force-fire a timer immediately
    Fire {
        /// Timer ID
        id: String,
    },

    /// Show daemon stats
    Stats {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }

    let (num_str, unit) = if s.ends_with("ms") {
        (&s[..s.len() - 2], "ms")
    } else if s.ends_with('s') {
        (&s[..s.len() - 1], "s")
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], "m")
    } else if s.ends_with('h') {
        (&s[..s.len() - 1], "h")
    } else if s.ends_with('d') {
        (&s[..s.len() - 1], "d")
    } else {
        // Default to seconds
        (s, "s")
    };

    let num: f64 = num_str.parse().map_err(|_| format!("invalid number: {}", num_str))?;

    let ms = match unit {
        "ms" => num,
        "s" => num * 1000.0,
        "m" => num * 60_000.0,
        "h" => num * 3_600_000.0,
        "d" => num * 86_400_000.0,
        _ => return Err(format!("unknown unit: {}", unit)),
    };

    Ok(ms as u64)
}

fn mcp_call(tool: &str, args: serde_json::Value) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool,
            "arguments": args
        }
    })
}

pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mcp_url = format!("{}/mcp", cli.url);

    match cli.command {
        Commands::Schedule {
            id,
            fire_in_ms,
            fire_at,
            r#in,
            priority,
            payload,
            repeat_ms,
            repeat,
            max_fires,
            ttl_ms,
            callback_url,
            tags,
        } => {
            let mut args = serde_json::Map::new();

            if let Some(id) = id {
                args.insert("id".into(), json!(id));
            }

            // Resolve fire time
            if let Some(dur) = r#in {
                args.insert("fire_in_ms".into(), json!(parse_duration(&dur)?));
            } else if let Some(ms) = fire_in_ms {
                args.insert("fire_in_ms".into(), json!(ms));
            } else if let Some(at) = fire_at {
                args.insert("fire_at".into(), json!(at));
            } else {
                return Err("one of --in, --fire-in-ms, or --fire-at required".into());
            }

            args.insert("priority".into(), json!(priority));

            if let Some(p) = payload {
                let v: serde_json::Value = serde_json::from_str(&p)?;
                args.insert("payload".into(), v);
            }

            if let Some(dur) = repeat {
                args.insert("repeat_ms".into(), json!(parse_duration(&dur)?));
            } else if let Some(ms) = repeat_ms {
                args.insert("repeat_ms".into(), json!(ms));
            }

            if let Some(mf) = max_fires {
                args.insert("max_fires".into(), json!(mf));
            }
            if let Some(ttl) = ttl_ms {
                args.insert("ttl_ms".into(), json!(ttl));
            }
            if let Some(url) = callback_url {
                args.insert("callback_url".into(), json!(url));
            }
            if !tags.is_empty() {
                args.insert("tags".into(), json!(tags));
            }

            let body = mcp_call("neuro_schedule", json!(args));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];
            println!("✅ Scheduled timer: {}", result["timer_id"].as_str().unwrap_or("?"));
            println!("   fires at: {}", result["fire_at"].as_str().unwrap_or("?"));
        }

        Commands::Cancel { id, tag } => {
            let mut args = serde_json::Map::new();
            if let Some(id) = id {
                args.insert("id".into(), json!(id));
            }
            if let Some(tag) = tag {
                args.insert("tag".into(), json!(tag));
            }

            let body = mcp_call("neuro_cancel", json!(args));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];
            let count = result["cancelled"].as_u64().unwrap_or(0);
            println!("🗑️  Cancelled {} timer(s)", count);
        }

        Commands::List { tag, limit, sort, json: as_json } => {
            let mut args = serde_json::Map::new();
            if let Some(tag) = tag {
                args.insert("tag".into(), json!(tag));
            }
            args.insert("limit".into(), json!(limit));
            args.insert("sort".into(), json!(sort));

            let body = mcp_call("neuro_list", json!(args));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];

            if as_json {
                println!("{}", serde_json::to_string_pretty(result)?);
                return Ok(());
            }

            let count = result["count"].as_u64().unwrap_or(0);
            println!("Active timers: {}\n", count);

            if let Some(timers) = result["timers"].as_array() {
                for t in timers {
                    let id = t["id"].as_str().unwrap_or("?");
                    let fire_at = t["fire_at"].as_str().unwrap_or("?");
                    let priority = t["priority"].as_u64().unwrap_or(0);
                    let fire_count = t["fire_count"].as_u64().unwrap_or(0);
                    let repeat = t["repeat_ms"].as_u64();
                    let tags: Vec<&str> = t["tags"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    let repeat_str = repeat
                        .map(|ms| format_duration(ms))
                        .unwrap_or_else(|| "once".into());

                    println!(
                        "  {} p={} fires={} {} [{}]",
                        id,
                        priority,
                        fire_count,
                        repeat_str,
                        tags.join(",")
                    );
                    println!("    next: {}", fire_at);
                }
            }
        }

        Commands::Get { id, json: as_json } => {
            let body = mcp_call("neuro_get", json!({"id": id}));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];

            if as_json {
                println!("{}", serde_json::to_string_pretty(result)?);
            } else {
                println!("Timer: {}", result["id"].as_str().unwrap_or("?"));
                println!("  fire_at:    {}", result["fire_at"].as_str().unwrap_or("?"));
                println!("  priority:   {}", result["priority"].as_u64().unwrap_or(0));
                println!("  fire_count: {}", result["fire_count"].as_u64().unwrap_or(0));
                if let Some(ms) = result["repeat_ms"].as_u64() {
                    println!("  repeat:     {}", format_duration(ms));
                }
                let tags: Vec<&str> = result["tags"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                if !tags.is_empty() {
                    println!("  tags:       {}", tags.join(", "));
                }
                println!("  payload:    {}", result["payload"]);
                println!("  created_at: {}", result["created_at"].as_str().unwrap_or("?"));
            }
        }

        Commands::Update {
            id,
            priority,
            fire_in_ms,
            r#in,
            repeat_ms,
            payload_merge,
            payload_replace,
            tags_add,
            tags_remove,
        } => {
            let mut args = serde_json::Map::new();
            args.insert("id".into(), json!(id));

            if let Some(p) = priority {
                args.insert("priority".into(), json!(p));
            }
            if let Some(dur) = r#in {
                args.insert("fire_in_ms".into(), json!(parse_duration(&dur)?));
            } else if let Some(ms) = fire_in_ms {
                args.insert("fire_in_ms".into(), json!(ms));
            }
            if let Some(ms) = repeat_ms {
                args.insert("repeat_ms".into(), json!(ms));
            }
            if let Some(p) = payload_merge {
                let v: serde_json::Value = serde_json::from_str(&p)?;
                args.insert("payload_merge".into(), v);
            }
            if let Some(p) = payload_replace {
                let v: serde_json::Value = serde_json::from_str(&p)?;
                args.insert("payload_replace".into(), v);
            }
            if !tags_add.is_empty() {
                args.insert("tags_add".into(), json!(tags_add));
            }
            if !tags_remove.is_empty() {
                args.insert("tags_remove".into(), json!(tags_remove));
            }

            let body = mcp_call("neuro_update", json!(args));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];
            let fields: Vec<&str> = result["updated"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            println!("✅ Updated timer: {}", result["timer_id"].as_str().unwrap_or("?"));
            println!("   fields: {}", fields.join(", "));
            println!("   next fire: {}", result["fire_at"].as_str().unwrap_or("?"));
        }

        Commands::Fire { id } => {
            let body = mcp_call("neuro_fire", json!({"id": id}));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];
            println!("🔥 Fired! count={} repeat={}", 
                result["fire_count"].as_u64().unwrap_or(0),
                result["will_repeat"].as_bool().unwrap_or(false));
        }

        Commands::Stats { json: as_json } => {
            let body = mcp_call("neuro_stats", json!({}));
            let resp: serde_json::Value = client.post(&mcp_url).json(&body).send().await?.json().await?;

            if let Some(err) = resp.get("error") {
                eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
                std::process::exit(1);
            }

            let result = &resp["result"];

            if as_json {
                println!("{}", serde_json::to_string_pretty(result)?);
                return Ok(());
            }

            let uptime = result["uptime_secs"].as_u64().unwrap_or(0);
            let days = uptime / 86400;
            let hours = (uptime % 86400) / 3600;
            let mins = (uptime % 3600) / 60;

            println!("=== Neuro Stats ===");
            println!("Uptime:        {}d {}h {}m", days, hours, mins);
            println!("Active timers: {}", result["active_timers"].as_u64().unwrap_or(0));
            println!("Total created: {}", result["total_created"].as_u64().unwrap_or(0));
            println!("Total fired:   {}", result["total_fired"].as_u64().unwrap_or(0));
            println!("Total cancel:  {}", result["total_cancelled"].as_u64().unwrap_or(0));
            println!("Fires/min:     {}", result["fires_last_minute"].as_u64().unwrap_or(0));
            println!("Fires/hour:    {}", result["fires_last_hour"].as_u64().unwrap_or(0));
            println!("Webhook OK:    {}", result["webhook_ok"].as_u64().unwrap_or(0));
            println!("Webhook fail:  {}", result["webhook_failed"].as_u64().unwrap_or(0));
            println!("Outbox:        {}", result["outbox_pending"].as_u64().unwrap_or(0));
        }
    }

    Ok(())
}

fn format_duration(ms: u64) -> String {
    if ms >= 86_400_000 {
        format!("every {}d", ms / 86_400_000)
    } else if ms >= 3_600_000 {
        format!("every {}h", ms / 3_600_000)
    } else if ms >= 60_000 {
        format!("every {}m", ms / 60_000)
    } else if ms >= 1000 {
        format!("every {}s", ms / 1000)
    } else {
        format!("every {}ms", ms)
    }
}
