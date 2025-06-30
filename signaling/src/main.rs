mod auth;
mod cluster;
mod messages;
mod rheomesh_sfu;
mod room;
mod server;

use anyhow::Result;
use clap::Parser;
use std::{env, sync::Arc};
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "webrtc-signaling")]
#[command(about = "A WebRTC signaling server")]
struct Args {
    #[arg(short, long, default_value = "0.0.0.0")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let args = Args::parse();

    let host = args.host;
    let port = env::var("PORT")
        .unwrap_or_else(|_| "9000".to_string())
        .parse::<u16>()
        .unwrap_or(9000);

    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET environment variable is required");

    // Determine whether to use clustering
    let cluster_mode = env::var("CLUSTER_MODE")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap_or(false);

    let room_manager = if cluster_mode {
        // Try to initialize cluster mode
        match initialize_cluster_mode().await {
            Ok(manager) => {
                info!("✅ Cluster mode enabled with Redis coordination");
                manager
            }
            Err(e) => {
                warn!("❌ Failed to initialize cluster mode: {}", e);
                warn!("🔄 Falling back to local mode");
                room::RoomManager::new()
            }
        }
    } else {
        info!("📍 Local mode enabled (clustering disabled)");
        room::RoomManager::new()
    };

    // Initialize Rheomesh SFU
    let sfu = match rheomesh_sfu::RheomeshSfu::new().await {
        Ok(sfu) => {
            info!("🚀 Rheomesh SFU initialized successfully");
            Some(Arc::new(sfu))
        }
        Err(e) => {
            warn!("Failed to initialize Rheomesh SFU: {}", e);
            warn!("Continuing without SFU - signaling only mode");
            None
        }
    };

    println!("Starting WebRTC signaling server on {}:{}", host, port);
    println!("JWT authentication enabled");

    server::start_server_with_room_manager_and_sfu(host, port, jwt_secret, room_manager, sfu).await
}

/// Initialize cluster mode with Redis
async fn initialize_cluster_mode(
) -> Result<room::RoomManager, Box<dyn std::error::Error + Send + Sync>> {
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

    let node_id = env::var("NODE_ID").unwrap_or_else(|_| {
        // Generate a unique node ID if not provided
        format!("signaling-{}", &uuid::Uuid::new_v4().to_string()[..8])
    });

    info!("Initializing cluster mode with Redis URL: {}", redis_url);
    info!("Node ID: {}", node_id);

    let cluster_manager = cluster::ClusterRoomManager::new(&redis_url, node_id).await?;
    let room_manager = room::RoomManager::with_implementation(Box::new(cluster_manager));

    Ok(room_manager)
}
