mod api;
mod bridge;
mod config;
mod library;
mod models;
mod openapi;
mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use actix_web::{App, HttpServer, web, middleware::Logger};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;
use tracing_subscriber::EnvFilter;

use crate::bridge::spawn_bridge_worker;
use crate::library::scan_library;
use crate::state::{AppState, PlayerStatus, QueueState, OutputState};

#[derive(Parser, Debug)]
#[command(name = "audio-server")]
struct Args {
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

    let cfg = match args.config.as_ref() {
        Some(path) => config::ServerConfig::load(path)?,
        None => {
            let auto_path = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|dir| dir.join("config.toml")));
            if let Some(path) = auto_path.as_ref() {
                if path.exists() {
                    config::ServerConfig::load(path)?
                } else {
                    return Err(anyhow::anyhow!(
                        "config file is required; use --config"
                    ));
                }
            } else {
                return Err(anyhow::anyhow!(
                    "config file is required; use --config"
                ));
            }
        }
    };
    let bind = match args.bind {
        Some(addr) => addr,
        None => config::bind_from_config(&cfg)?.unwrap_or_else(|| "0.0.0.0:8080".parse().expect("default bind")),
    };
    let media_dir = match args.media_dir {
        Some(dir) => dir,
        None => config::media_dir_from_config(&cfg)?,
    };
    tracing::info!(
        bind = %bind,
        media_dir = %media_dir.display(),
        "starting audio-server"
    );
    let library = scan_library(&media_dir)?;
    let (outputs, active_id, bridge_addr) = config::outputs_from_config(cfg)?;

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
    let outputs = Arc::new(Mutex::new(OutputState {
        active_id,
        outputs,
    }));
    spawn_bridge_worker(bridge_addr, cmd_rx, cmd_tx.clone(), status.clone(), queue.clone());

    let state = web::Data::new(AppState::new(library, cmd_tx, status, queue, outputs));

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(Logger::default().exclude("/status").exclude("/queue"))
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-doc/openapi.json", openapi::ApiDoc::openapi()),
            )
            .service(api::list_library)
            .service(api::rescan_library)
            .service(api::play_track)
            .service(api::pause_toggle)
            .service(api::queue_list)
            .service(api::queue_add)
            .service(api::queue_remove)
            .service(api::queue_clear)
            .service(api::queue_next)
            .service(api::status)
            .service(api::outputs_list)
            .service(api::outputs_select)
            .service(api::output_devices)
            .service(api::output_set_device)
    })
    .bind(bind)?
    .run()
    .await?;

    Ok(())
}
