mod api;
mod bridge;
mod bridge_manager;
mod config;
mod discovery;
mod library;
mod models;
mod openapi;
mod output_controller;
mod output_providers;
mod playback_transport;
mod queue_playback;
mod local_player;
mod startup;
mod status_store;
mod state;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use std::path::PathBuf;

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_SHA"),
    ", ",
    env!("BUILD_DATE"),
    ")"
);

#[derive(Parser, Debug)]
#[command(name = "audio-hub-server", version = VERSION)]
pub(crate) struct Args {
    /// HTTP bind address, e.g. 0.0.0.0:8080
    #[arg(long)]
    bind: Option<std::net::SocketAddr>,

    /// Media library root directory
    #[arg(long)]
    media_dir: Option<PathBuf>,

    /// Optional server config file (TOML)
    #[arg(long)]
    config: Option<PathBuf>,
}

#[actix_web::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,actix_web=info,audio_server=info")
        }))
        .init();

    startup::run(args).await
}
