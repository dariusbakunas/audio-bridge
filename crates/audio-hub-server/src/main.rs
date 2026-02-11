//! HTTP control + streaming server for the audio hub.
//!
//! Scans the media library, manages output providers, and serves playback control APIs.

mod api;
mod bridge;
mod bridge_device_streams;
mod bridge_manager;
mod bridge_transport;
mod browser;
mod config;
mod cover_art;
mod discovery;
mod events;
mod library;
mod models;
mod metadata_db;
mod metadata_service;
mod musicbrainz;
mod openapi;
mod output_controller;
mod output_providers;
mod playback_transport;
mod playback_manager;
mod queue_service;
mod local_player;
mod startup;
mod status_store;
mod state;
mod tag_writer;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use std::path::PathBuf;

use crate::events::{LogBus, LogLayer};

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

    let log_bus = std::sync::Arc::new(LogBus::new(500));
    let log_layer = LogLayer::new(log_bus.clone());
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,actix_web=info,audio_server=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(log_layer)
        .init();

    tracing::info!(
        version = VERSION,
        bind = ?args.bind,
        media_dir = ?args.media_dir,
        config = ?args.config,
        "audio-hub-server starting"
    );

    startup::run(args, log_bus).await
}
