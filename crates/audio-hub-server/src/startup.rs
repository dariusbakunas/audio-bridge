use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use actix_web::{App, HttpServer, web, middleware::Logger};
use anyhow::Result;
use crossbeam_channel::unbounded;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::api;
use crate::bridge::{http_list_devices, http_set_device};
use crate::bridge_manager::parse_output_id;
use crate::config;
use crate::discovery::{spawn_discovered_health_watcher, spawn_mdns_discovery};
use crate::library::scan_library;
use crate::openapi;
use crate::state::{AppState, BridgeState, PlayerStatus, QueueState};

pub(crate) async fn run(args: crate::Args) -> Result<()> {
    let cfg = load_config(args.config.as_ref())?;
    let bind = resolve_bind(args.bind, &cfg)?;
    let public_base_url = config::public_base_url_from_config(&cfg, bind)?;
    let media_dir = resolve_media_dir(args.media_dir, &cfg)?;
    tracing::info!(
        bind = %bind,
        public_base_url = %public_base_url,
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
    let (active_bridge_id, active_output_id) =
        resolve_active_output(&cfg, &bridges, &mut device_to_set)?;
    let active_http_addr = active_bridge_id.as_ref().and_then(|bridge_id| {
        bridges
            .iter()
            .find(|b| b.id == *bridge_id)
            .map(|b| b.http_addr)
    });

    let (cmd_tx, _cmd_rx) = unbounded();
    let status = Arc::new(Mutex::new(PlayerStatus::default()));
    let queue = Arc::new(Mutex::new(QueueState::default()));
    let bridge_online = Arc::new(AtomicBool::new(false));
    let bridges_state = Arc::new(Mutex::new(BridgeState {
        bridges,
        active_bridge_id: active_bridge_id.clone(),
        active_output_id: active_output_id.clone(),
    }));
    let discovered_bridges = Arc::new(Mutex::new(std::collections::HashMap::new()));
    if let (Some(device_name), Some(http_addr)) = (device_to_set, active_http_addr) {
        let _ = http_set_device(http_addr, &device_name);
    }

    let state = web::Data::new(AppState::new(
        library,
        cmd_tx,
        status,
        queue,
        bridges_state,
        bridge_online.clone(),
        discovered_bridges.clone(),
        public_base_url,
    ));
    setup_shutdown(state.player.clone());
    spawn_mdns_discovery(state.clone());
    spawn_discovered_health_watcher(state.clone());
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(Logger::default().exclude("/status").exclude("/queue").exclude("/stream"))
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-doc/openapi.json", openapi::ApiDoc::openapi()),
            )
            .service(api::list_library)
            .service(api::rescan_library)
            .service(api::play_track)
            .service(api::pause_toggle)
            .service(api::seek)
            .service(api::stream_track)
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

fn load_config(path: Option<&PathBuf>) -> Result<config::ServerConfig> {
    match path {
        Some(path) => config::ServerConfig::load(path),
        None => {
            let auto_path = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|dir| dir.join("config.toml")));
            if let Some(path) = auto_path.as_ref() {
                if path.exists() {
                    config::ServerConfig::load(path)
                } else {
                    Err(anyhow::anyhow!(
                        "config file is required; use --config"
                    ))
                }
            } else {
                Err(anyhow::anyhow!(
                    "config file is required; use --config"
                ))
            }
        }
    }
}

fn resolve_bind(
    bind: Option<std::net::SocketAddr>,
    cfg: &config::ServerConfig,
) -> Result<std::net::SocketAddr> {
    Ok(match bind {
        Some(addr) => addr,
        None => config::bind_from_config(cfg)?
            .unwrap_or_else(|| "0.0.0.0:8080".parse().expect("default bind")),
    })
}

fn resolve_media_dir(
    dir: Option<PathBuf>,
    cfg: &config::ServerConfig,
) -> Result<PathBuf> {
    Ok(match dir {
        Some(dir) => dir,
        None => config::media_dir_from_config(cfg)?,
    })
}

fn resolve_active_output(
    cfg: &config::ServerConfig,
    bridges: &[crate::config::BridgeConfigResolved],
    device_to_set: &mut Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    let result = match cfg.active_output.as_ref() {
        Some(id) => match parse_output_id(id) {
            Ok((bridge_id, device_name)) => {
                let active_id = format!("bridge:{}:{}", bridge_id, device_name);
                *device_to_set = Some(device_name);
                (Some(bridge_id), Some(active_id))
            }
            Err(e) => return Err(anyhow::anyhow!(e)),
        },
        None => {
            if bridges.is_empty() {
                tracing::warn!("no configured bridges; starting without active output");
                (None, None)
            } else {
                let mut first_bridge: Option<crate::config::BridgeConfigResolved> = None;
                let mut found_active: Option<(String, String)> = None;
                for bridge in bridges {
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
                            *device_to_set = Some(device.name.clone());
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
                    (Some(found.0), Some(found.1))
                } else {
                    let bridge = first_bridge.unwrap();
                    tracing::warn!(
                        bridge_id = %bridge.id,
                        bridge_name = %bridge.name,
                        "active_output not set; no outputs available, starting without active output"
                    );
                    (None, None)
                }
            }
        }
    };
    Ok(result)
}

fn setup_shutdown(player: std::sync::Arc<std::sync::Mutex<crate::bridge::BridgePlayer>>) {
    let _ = ctrlc::set_handler(move || {
        if let Ok(player) = player.lock() {
            let _ = player.cmd_tx.send(crate::bridge::BridgeCommand::Quit);
        }
        if let Some(system) = actix_web::rt::System::try_current() {
            system.stop();
        } else {
            std::process::exit(0);
        }
    });
}
