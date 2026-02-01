mod api;
mod bridge;
mod library;
mod models;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use actix_web::{App, HttpServer, web, middleware::Logger};
use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;
use tracing_subscriber::EnvFilter;

use crate::bridge::spawn_bridge_worker;
use crate::library::scan_library;
use crate::state::{AppState, PlayerStatus, QueueState};

#[derive(Parser, Debug)]
#[command(name = "audio-server")]
struct Args {
    /// HTTP bind address, e.g. 0.0.0.0:8080
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: SocketAddr,

    /// Media library root directory
    #[arg(long)]
    media_dir: PathBuf,

    /// Bridge receiver address (Pi), e.g. 192.168.1.50:5555
    #[arg(long)]
    bridge: SocketAddr,
}

#[actix_web::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,actix_web=info,audio_server=info")
        }))
        .init();

    tracing::info!(
        bind = %args.bind,
        media_dir = %args.media_dir.display(),
        bridge = %args.bridge,
        "starting audio-server"
    );

    let library = scan_library(&args.media_dir)?;

    let (cmd_tx, cmd_rx) = unbounded();
    let shutdown_tx = cmd_tx.clone();
    let _ = ctrlc::set_handler(move || {
        let _ = shutdown_tx.send(crate::bridge::BridgeCommand::Quit);
        if let Some(system) = actix_web::rt::System::try_current() {
            system.stop();
        } else {
            std::process::exit(0);
        }
    });
    let status = Arc::new(Mutex::new(PlayerStatus::default()));
    let queue = Arc::new(Mutex::new(QueueState::default()));
    spawn_bridge_worker(args.bridge, cmd_rx, status.clone(), queue.clone());

    let state = web::Data::new(AppState::new(library, cmd_tx, status, queue));

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(Logger::default().exclude("/status").exclude("/queue"))
            .service(api::list_library)
            .service(api::rescan_library)
            .service(api::play_track)
            .service(api::pause_toggle)
            .service(api::next_track)
            .service(api::queue_list)
            .service(api::queue_add)
            .service(api::queue_remove)
            .service(api::queue_clear)
            .service(api::queue_next)
            .service(api::queue_replace_play)
            .service(api::status)
    })
    .bind(args.bind)?
    .run()
    .await?;

    Ok(())
}
