pub mod config;
pub mod delivery;
pub mod engine;
mod mcp;
pub mod persistence;
pub mod types;

use crate::config::LimitsConfig;

use clap::Parser;
use config::Config;
use delivery::Delivery;
use engine::Engine;
use mcp::create_mcp_server;
use persistence::Persistence;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "neuro")]
#[command(about = "Programmable timer engine with MCP interface")]
struct Args {
    #[arg(short, long, default_value = "neuro.toml")]
    config: PathBuf,

    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[arg(long)]
    version: bool,
}

fn main() {
    let args = Args::parse();

    if args.version {
        println!("neuro v{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Setup logging
    let log_level = match args.verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // Load config
    let config = match Config::load(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    // Ensure state directory exists
    if let Err(e) = std::fs::create_dir_all(&config.daemon.state_dir) {
        eprintln!("Failed to create state directory: {}", e);
        std::process::exit(1);
    }

    // Run the async runtime
    let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
    rt.block_on(async {
        if let Err(e) = run(config).await {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    });
}

async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting neuro daemon");

    // Initialize persistence and load state
    let persistence = Persistence::new(config.daemon.state_dir.clone());
    let timers = persistence.boot().await?;

    // Initialize engine with loaded timers
    let engine = Engine::new(config.limits.clone());
    let mut engine = engine;
    engine.load_timers(timers);

    let engine = Arc::new(Mutex::new(engine));

    // Initialize delivery
    let delivery = Delivery::new(config.delivery.clone());

    // Create MCP server
    let app = create_mcp_server(
        engine.clone(),
        delivery,
        persistence.clone(),
        config.delivery.clone(),
    );

    // Start HTTP server
    let addr = config.daemon.listen;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Listening on {}", addr);

    let server = axum::serve(listener, app);

    // Start tick loop in background
    let tick_engine = engine.clone();
    let tick_persistence = persistence.clone();
    let tick_delivery = Arc::new(Delivery::new(config.delivery.clone()));
    let tick_config = config.clone();
    let tick_handle = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(tick_config.daemon.tick_ms));
        let mut snapshot_timer = interval(Duration::from_secs(tick_config.persistence.snapshot_every_secs));

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let events = {
                        let mut eng = tick_engine.lock().await;
                        eng.tick()
                    };

                    // Deliver events
                    for event in events {
                        let outbox_path = tick_persistence.state_dir().join("outbox.jsonl");
                        let url = tick_delivery.default_url().to_string();

                        let result = tick_delivery.deliver(&event, &url, &outbox_path).await;

                        let mut eng = tick_engine.lock().await;
                        if result.is_ok() {
                            eng.record_webhook_success();
                        } else {
                            eng.record_webhook_failure();
                        }
                    }
                }
                _ = snapshot_timer.tick() => {
                    let should_snapshot = {
                        let eng = tick_engine.lock().await;
                        eng.ops_since_snapshot() >= tick_config.persistence.snapshot_every_ops
                    };

                    if should_snapshot {
                        let timer_ids = {
                            let eng = tick_engine.lock().await;
                            eng.list(None, 100000, types::SortBy::FireAt)
                                .iter()
                                .map(|s| s.id.clone())
                                .collect::<Vec<_>>()
                        };

                        let timers = {
                            let eng = tick_engine.lock().await;
                            timer_ids
                                .iter()
                                .filter_map(|id| eng.get(id).cloned())
                                .collect::<Vec<_>>()
                        };

                        if let Err(e) = tick_persistence.write_snapshot(&timers).await {
                            error!("Failed to write snapshot: {}", e);
                        } else {
                            let mut eng = tick_engine.lock().await;
                            eng.reset_ops_since_snapshot();
                            let _ = tick_persistence.truncate_wal().await;
                        }
                    }
                }
            }
        }
    });

    // Run server with graceful shutdown
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                error!("Server error: {}", e);
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl-C, shutting down");
        }
        _ = async {
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::terminate()).unwrap().recv().await
        } => {
            info!("Received SIGTERM, shutting down");
        }
    }

    // Graceful shutdown
    tick_handle.abort();

    // Write final snapshot
    let timer_ids = {
        let eng = engine.lock().await;
        eng.list(None, 100000, types::SortBy::FireAt)
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>()
    };

    let timers = {
        let eng = engine.lock().await;
        timer_ids
            .iter()
            .filter_map(|id| eng.get(id).cloned())
            .collect::<Vec<_>>()
    };

    if let Err(e) = persistence.write_snapshot(&timers).await {
        error!("Failed to write final snapshot: {}", e);
    }

    info!("Shutdown complete");
    Ok(())
}
