mod api;
mod bridge;
mod library;
mod models;
mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use actix_web::{App, HttpServer, web};
use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;

use crate::bridge::spawn_bridge_worker;
use crate::library::scan_library;
use crate::state::{AppState, PlayerStatus};

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

    let library = scan_library(&args.media_dir)?;

    let (cmd_tx, cmd_rx) = unbounded();
    let status = Arc::new(Mutex::new(PlayerStatus::default()));
    spawn_bridge_worker(args.bridge, cmd_rx, status.clone());

    let state = web::Data::new(AppState::new(library, cmd_tx, status));

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(api::list_library)
            .service(api::rescan_library)
            .service(api::play_track)
            .service(api::pause_toggle)
            .service(api::next_track)
            .service(api::status)
    })
    .bind(args.bind)?
    .run()
    .await?;

    Ok(())
}
