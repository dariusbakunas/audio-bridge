mod api;
mod bridge;
mod config;
mod library;
mod models;
mod openapi;
mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;

use actix_web::{App, HttpServer, web, middleware::Logger};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use anyhow::Result;
use clap::Parser;
use crossbeam_channel::unbounded;
use tracing_subscriber::EnvFilter;

use crate::bridge::{http_list_devices, http_set_device};
use crate::library::scan_library;
use crate::state::{AppState, BridgeState, PlayerStatus, QueueState};

#[derive(Parser, Debug)]
#[command(name = "audio-hub-server")]
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
        "starting audio-hub-server"
    );
    let library = scan_library(&media_dir)?;
    let bridges = config::bridges_from_config(&cfg)?;
    tracing::info!(
        count = bridges.len(),
        ids = ?bridges.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
        "loaded bridges from config"
    );
    let mut device_to_set: Option<String> = None;
    let (active_bridge_id, active_output_id) = match cfg.active_output.as_ref() {
        Some(id) => match parse_output_id(id) {
            Ok((bridge_id, device_name)) => {
                let active_id = format!("bridge:{}:{}", bridge_id, device_name);
                device_to_set = Some(device_name);
                (bridge_id, active_id)
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        },
        None => {
            let mut first_bridge: Option<crate::config::BridgeConfigResolved> = None;
            let mut found_active: Option<(String, String)> = None;
            for bridge in &bridges {
                if first_bridge.is_none() {
                    first_bridge = Some(bridge.clone());
                }
            match http_list_devices(bridge.http_addr) {
                Ok(devices) if !devices.is_empty() => {
                    let device = devices[0].clone();
                    let active_id = format!("bridge:{}:{}", bridge.id, device.name);
                        tracing::info!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            device = %device.name,
                            output_id = %active_id,
                            "active_output not set; defaulting to first available output"
                        );
                        device_to_set = Some(device.name.clone());
                        found_active = Some((bridge.id.clone(), active_id));
                        break;
                    }
                    Ok(_) => {
                        tracing::warn!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            "bridge returned no outputs while selecting default"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            error = %e,
                            "bridge unavailable while selecting default"
                        );
                    }
                }
            }

            if let Some(found) = found_active {
                found
            } else {
                let bridge = first_bridge.ok_or_else(|| anyhow::anyhow!("config must define at least one bridge"))?;
                let pending = format!("bridge:{}:pending", bridge.id);
                tracing::warn!(
                    bridge_id = %bridge.id,
                    bridge_name = %bridge.name,
                    output_id = %pending,
                    "active_output not set; no bridges available, starting with pending output"
                );
                (bridge.id.clone(), pending)
            }
        }
    };
    let (_active_addr, active_http_addr) = {
        let bridge = bridges
            .iter()
            .find(|b| b.id == active_bridge_id)
            .ok_or_else(|| anyhow::anyhow!("active bridge id not found"))?
            .clone();
        (bridge.addr, bridge.http_addr)
    };

    let (cmd_tx, _cmd_rx) = unbounded();
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
    let bridge_online = Arc::new(AtomicBool::new(false));
    let bridges_state = Arc::new(Mutex::new(BridgeState {
        bridges,
        active_bridge_id: active_bridge_id.clone(),
        active_output_id: active_output_id.clone(),
    }));
    if let Some(device_name) = device_to_set {
        let _ = http_set_device(active_http_addr, &device_name);
    }

    let state = web::Data::new(AppState::new(
        library,
        cmd_tx,
        status,
        queue,
        bridges_state,
        bridge_online.clone(),
    ));
    spawn_pending_output_watcher(state.clone());

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
            .service(api::bridges_list)
            .service(api::bridge_outputs_list)
            .service(api::outputs_list)
            .service(api::outputs_select)
    })
    .bind(bind)?
    .run()
    .await?;

    Ok(())
}

fn parse_output_id(id: &str) -> Result<(String, String), String> {
    let mut parts = id.splitn(3, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    let device = parts.next().unwrap_or("");
    if kind != "bridge" || bridge_id.is_empty() || device.is_empty() {
        return Err("invalid output id".to_string());
    }
    Ok((bridge_id.to_string(), device.to_string()))
}

fn is_pending_output(id: &str) -> bool {
    id.ends_with(":pending")
}

fn spawn_pending_output_watcher(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        let mut backoff_ms = 500u64;
        loop {
            let (pending, active_bridge_id, active_output_id, bridges) = {
                let bridges = state.bridges.lock().unwrap();
                (
                    is_pending_output(&bridges.active_output_id),
                    bridges.active_bridge_id.clone(),
                    bridges.active_output_id.clone(),
                    bridges.bridges.clone(),
                )
            };

            if !pending {
                backoff_ms = 500;
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }

            let mut resolved = false;
            for bridge in &bridges {
                match http_list_devices(bridge.http_addr) {
                    Ok(devices) if !devices.is_empty() => {
                        let device = devices[0].clone();
                        let output_id = format!("bridge:{}:{}", bridge.id, device.name);
                        match http_set_device(bridge.http_addr, &device.name) {
                            Ok(()) => {
                                {
                                    let mut bridges_state = state.bridges.lock().unwrap();
                                    if bridges_state.active_output_id == active_output_id {
                                        bridges_state.active_output_id = output_id.clone();
                                        bridges_state.active_bridge_id = bridge.id.clone();
                                    }
                                }
                                tracing::info!(
                                    bridge_id = %bridge.id,
                                    bridge_name = %bridge.name,
                                    device = %device.name,
                                    output_id = %output_id,
                                    "active output resolved from pending"
                                );
                                state
                                    .bridge_online
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                                resolved = true;
                                backoff_ms = 500;
                                break;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    bridge_id = %bridge.id,
                                    bridge_name = %bridge.name,
                                    error = %e,
                                    "device available but bridge command failed; retrying"
                                );
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::debug!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            "bridge returned no outputs while pending"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            bridge_id = %bridge.id,
                            bridge_name = %bridge.name,
                            error = %e,
                            "bridge unavailable while pending; retrying"
                        );
                    }
                }
            }

            if !resolved {
                std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms * 2).min(30_000);
            }
        }
    });
}
