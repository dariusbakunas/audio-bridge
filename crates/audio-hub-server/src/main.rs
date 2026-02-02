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
use mdns_sd::{ServiceDaemon, ServiceEvent};

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
            if bridges.is_empty() {
                tracing::warn!("no configured bridges; starting with pending output");
                ("pending".to_string(), "bridge:pending:pending".to_string())
            } else {
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
                    let bridge = first_bridge.unwrap();
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
        }
    };
    let active_http_addr = bridges
        .iter()
        .find(|b| b.id == active_bridge_id)
        .map(|b| b.http_addr);

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
    ));
    spawn_mdns_discovery(state.clone());
    spawn_discovered_health_watcher(state.clone());
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

fn spawn_mdns_discovery(state: web::Data<AppState>) {
    std::thread::spawn(move || {
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: failed to start daemon");
                return;
            }
        };
        let receiver = match daemon.browse("_audio-bridge._tcp.local.") {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "mdns: browse failed");
                return;
            }
        };
        tracing::info!("mdns: browsing for _audio-bridge._tcp.local.");
        let mut fullname_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for event in receiver {
            match event {
                ServiceEvent::ServiceFound(_ty, fullname) => {
                    tracing::info!(fullname = %fullname, "mdns: service found");
                    if let Some(id) = fullname_to_id.get(&fullname).cloned() {
                        if let Ok(mut map) = state.discovered_bridges.lock() {
                            if let Some(entry) = map.get_mut(&id) {
                                entry.last_seen = std::time::Instant::now();
                            }
                        }
                    }
                }
                ServiceEvent::ServiceResolved(info) => {
                    tracing::info!(
                        fullname = %info.get_fullname(),
                        host = %info.get_hostname(),
                        port = info.get_port(),
                        "mdns: service resolved"
                    );
                    let id = info
                        .get_property("id")
                        .map(|p| p.val_str().to_string())
                        .map(|s| s.strip_prefix("id=").unwrap_or(&s).to_string())
                        .unwrap_or_else(|| info.get_fullname().to_string());
                    let name = info
                        .get_property("name")
                        .map(|p| p.val_str().to_string())
                        .map(|s| s.strip_prefix("name=").unwrap_or(&s).to_string())
                        .unwrap_or_else(|| id.clone());
                    let api_port = info
                        .get_property("api_port")
                        .and_then(|p| p.val_str().parse::<u16>().ok());
                    let addr = info
                        .get_addresses()
                        .iter()
                        .find_map(|ip| if let std::net::IpAddr::V4(v4) = ip { Some(*v4) } else { None });
                    let Some(ip) = addr else {
                        tracing::warn!(fullname = %info.get_fullname(), "mdns: resolved without IPv4");
                        continue;
                    };
                    let stream_port = info.get_port();
                    let stream = std::net::SocketAddr::new(std::net::IpAddr::V4(ip), stream_port);
                    let http_port = api_port.unwrap_or_else(|| stream_port.saturating_add(1));
                    let http = std::net::SocketAddr::new(std::net::IpAddr::V4(ip), http_port);
                    let bridge = crate::config::BridgeConfigResolved {
                        id: id.clone(),
                        name,
                        addr: stream,
                        http_addr: http,
                    };
                    if let Ok(mut map) = state.discovered_bridges.lock() {
                        let now = std::time::Instant::now();
                        map.insert(
                            id.clone(),
                            crate::state::DiscoveredBridge {
                                bridge,
                                last_seen: now,
                            },
                        );
                    }
                    tracing::info!(
                        bridge_id = %id,
                        addr = %stream,
                        http_addr = %http,
                        "mdns: discovered bridge"
                    );
                    fullname_to_id.insert(info.get_fullname().to_string(), id);
                }
                ServiceEvent::ServiceRemoved(name, _) => {
                    if let Some(id) = fullname_to_id.remove(&name) {
                        if let Ok(mut map) = state.discovered_bridges.lock() {
                            map.remove(&id);
                        }
                        tracing::info!(bridge_id = %id, "mdns: bridge removed");
                    }
                }
                _ => {}
            }
        }
    });
}

fn spawn_discovered_health_watcher(state: web::Data<AppState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(15));
        let snapshot = match state.discovered_bridges.lock() {
            Ok(map) => map
                .iter()
                .map(|(id, entry)| (id.clone(), entry.bridge.http_addr, entry.last_seen))
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };

        let now = std::time::Instant::now();
        for (id, http_addr, last_seen) in snapshot {
            let ok = ping_bridge(http_addr);
            if ok {
                if let Ok(mut map) = state.discovered_bridges.lock() {
                    if let Some(entry) = map.get_mut(&id) {
                        entry.last_seen = now;
                    }
                }
            } else if now.duration_since(last_seen) > std::time::Duration::from_secs(60) {
                if let Ok(mut map) = state.discovered_bridges.lock() {
                    map.remove(&id);
                }
                tracing::info!(bridge_id = %id, "mdns: bridge removed (health check)");
            }
        }
    });
}

fn ping_bridge(http_addr: std::net::SocketAddr) -> bool {
    let url = format!("http://{http_addr}/health");
    let resp = ureq::get(&url).timeout(std::time::Duration::from_secs(2)).call();
    resp.map(|r| r.status() / 100 == 2).unwrap_or(false)
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
                let discovered = state.discovered_bridges.lock().unwrap();
                let mut merged = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for b in bridges.bridges.iter() {
                    seen.insert(b.id.clone());
                    merged.push(b.clone());
                }
                for (id, b) in discovered.iter() {
                    if !seen.contains(id) {
                        merged.push(b.bridge.clone());
                    }
                }
                (
                    is_pending_output(&bridges.active_output_id),
                    bridges.active_bridge_id.clone(),
                    bridges.active_output_id.clone(),
                    merged,
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
