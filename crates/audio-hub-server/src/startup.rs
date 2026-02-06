//! Actix server startup + app wiring.
//!
//! Builds the shared state, routes, middleware, and OpenAPI endpoints.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use actix_cors::Cors;
use actix_files::{Files, NamedFile};
use actix_web::{App, HttpServer, web};
use actix_web::dev::{Service, ServiceRequest, ServiceResponse};
use actix_web::Error;
use futures_util::future::{ok, LocalBoxFuture, Ready};
use std::task::{Context, Poll};
use anyhow::Result;
use crossbeam_channel::unbounded;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::api;
use crate::bridge_transport::BridgeTransportClient;
use crate::bridge_manager::parse_output_id;
use crate::config;
use crate::discovery::{spawn_discovered_health_watcher, spawn_mdns_discovery};
use crate::library::scan_library;
use crate::openapi;
use crate::state::{AppState, BridgeProviderState, BridgeState, LocalProviderState, PlayerStatus, QueueState};

/// Build server state and start the Actix HTTP server.
pub(crate) async fn run(args: crate::Args) -> Result<()> {
    let cfg = load_config(args.config.as_ref())?;
    let bind = resolve_bind(args.bind, &cfg)?;
    let public_base_url = config::public_base_url_from_config(&cfg, bind)?;
    let media_dir = resolve_media_dir(args.media_dir, &cfg)?;
    let web_ui_dist = locate_web_ui_dist();
    tracing::info!(
        bind = %bind,
        public_base_url = %public_base_url,
        media_dir = %media_dir.display(),
        "starting audio-hub-server"
    );
    if let Some(dist) = web_ui_dist.as_ref() {
        tracing::info!(path = %dist.display(), "web ui static assets enabled");
    } else {
        tracing::info!("web ui static assets disabled (web-ui/dist not found)");
    }
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
        let _ = BridgeTransportClient::new(http_addr, public_base_url.clone())
            .set_device(&device_name);
    }

    let bridge_state = Arc::new(BridgeProviderState::new(
        cmd_tx,
        bridges_state,
        bridge_online.clone(),
        discovered_bridges.clone(),
        public_base_url,
    ));
    let events = crate::events::EventBus::new();
    let status_store = crate::status_store::StatusStore::new(status, events.clone());
    let queue_service = crate::queue_service::QueueService::new(queue, status_store.clone(), events.clone());
    let playback_manager = crate::playback_manager::PlaybackManager::new(
        bridge_state.player.clone(),
        status_store,
        queue_service,
    );
    let local_enabled = cfg.local_outputs.unwrap_or(false);
    let local_id = cfg
        .local_id
        .clone()
        .unwrap_or_else(|| "local".to_string());
    let local_name = cfg
        .local_name
        .clone()
        .unwrap_or_else(|| "Local Host".to_string());
    let local_device = cfg.local_device.clone().filter(|s| !s.trim().is_empty());
    if local_enabled {
        let host = cpal::default_host();
        match audio_player::device::list_device_infos(&host) {
            Ok(devices) => {
                if devices.is_empty() {
                    tracing::warn!("local outputs enabled but no devices were found");
                } else {
                    tracing::info!(
                        count = devices.len(),
                        names = ?devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
                        "local output devices detected"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "local outputs enabled but device enumeration failed");
            }
        }
    }
    let (local_cmd_tx, _local_cmd_rx) = unbounded();
    let local_state = Arc::new(LocalProviderState {
        enabled: local_enabled,
        id: local_id,
        name: local_name,
        player: Arc::new(Mutex::new(crate::bridge::BridgePlayer { cmd_tx: local_cmd_tx })),
        running: Arc::new(AtomicBool::new(false)),
    });
    let device_selection = crate::state::DeviceSelectionState {
        local: Arc::new(Mutex::new(local_device.clone())),
        bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
    };
    let state = web::Data::new(AppState::new(
        library,
        bridge_state,
        local_state,
        playback_manager,
        device_selection,
        events,
    ));
    setup_shutdown(state.bridge.player.clone());
    spawn_mdns_discovery(state.clone());
    spawn_discovered_health_watcher(state.clone());
    HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:5173")
            .allowed_origin("http://127.0.0.1:5173")
            .allowed_methods(vec!["GET", "POST", "HEAD"])
            .allowed_headers(vec![actix_web::http::header::CONTENT_TYPE])
            .max_age(3600);

        let mut app = App::new()
            .app_data(state.clone())
            .wrap(cors)
            .wrap(FilteredLogger)
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}")
                    .url("/api-doc/openapi.json", openapi::ApiDoc::openapi()),
            )
            .service(api::list_library)
            .service(api::rescan_library)
            .service(api::play_track)
            .service(api::pause_toggle)
            .service(api::stop)
            .service(api::seek)
            .service(api::stream_track)
            .service(api::queue_list)
            .service(api::queue_add)
            .service(api::queue_add_next)
            .service(api::queue_remove)
            .service(api::queue_clear)
            .service(api::queue_next)
            .service(api::queue_stream)
            .service(api::status_for_output)
            .service(api::status_stream)
            .service(api::providers_list)
            .service(api::provider_outputs_list)
            .service(api::outputs_list)
            .service(api::outputs_stream)
            .service(api::outputs_select);

        if let Some(dist) = web_ui_dist.clone() {
            let assets_dir = dist.join("assets");
            if assets_dir.exists() {
                app = app.service(Files::new("/assets", assets_dir));
            }

            let index_path = dist.join("index.html");
            if index_path.exists() {
                let index_root = index_path.clone();
                let index_html = index_path.clone();
                app = app
                    .service(
                        web::resource("/")
                            .route(web::get().to(move || serve_index(index_root.clone()))),
                    )
                    .service(
                        web::resource("/index.html")
                            .route(web::get().to(move || serve_index(index_html.clone()))),
                    );
            }
        }

        app
    })
    .bind(bind)?
    .run()
    .await?;

    Ok(())
}

/// Return true when the request path should be logged.
fn should_log_path(path: &str) -> bool {
    if path == "/queue" || path == "/stream" || path == "/queue/stream" || path.ends_with("/status/stream") {
        return false;
    }
    if path.starts_with("/outputs/") && path != "/outputs/select" {
        return false;
    }
    true
}

/// Actix middleware that filters noisy paths from logging.
struct FilteredLogger;

impl<S, B> actix_web::dev::Transform<S, ServiceRequest> for FilteredLogger
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = FilteredLoggerMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(FilteredLoggerMiddleware { service })
    }
}

/// Service wrapper that applies the logging filter.
struct FilteredLoggerMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for FilteredLoggerMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let path = req.path().to_string();
        let should_log = should_log_path(&path);
        let method = req.method().clone();
        let peer = req.connection_info().realip_remote_addr().unwrap_or("-").to_string();
        let ua = req
            .headers()
            .get("User-Agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        let start = std::time::Instant::now();
        let fut = self.service.call(req);
        Box::pin(async move {
            let res = fut.await?;
            if should_log {
                tracing::info!(
                    method = %method,
                    path = %path,
                    status = %res.status().as_u16(),
                    user_agent = %ua,
                    peer = %peer,
                    elapsed_ms = %start.elapsed().as_millis(),
                    "http request"
                );
            }
            Ok(res)
        })
    }
}

/// Load server config from disk or return defaults.
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

/// Resolve the final bind address from args + config.
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

/// Resolve the media directory from args + config.
fn resolve_media_dir(
    dir: Option<PathBuf>,
    cfg: &config::ServerConfig,
) -> Result<PathBuf> {
    Ok(match dir {
        Some(dir) => dir,
        None => config::media_dir_from_config(cfg)?,
    })
}

fn locate_web_ui_dist() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(dir) = std::env::current_dir() {
        candidates.push(dir.join("web-ui").join("dist"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("web-ui").join("dist"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

async fn serve_index(index_path: PathBuf) -> actix_web::Result<NamedFile> {
    Ok(NamedFile::open(index_path)?)
}

/// Resolve active output id from config and available bridges.
fn resolve_active_output(
    cfg: &config::ServerConfig,
    bridges: &[crate::config::BridgeConfigResolved],
    device_to_set: &mut Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    let result = match cfg.active_output.as_ref() {
        Some(id) if id.starts_with("local:") => (None, Some(id.clone())),
        Some(id) => match parse_output_id(id) {
            Ok((bridge_id, device_id)) => {
                let http_addr = bridges
                    .iter()
                    .find(|b| b.id == bridge_id)
                    .map(|b| b.http_addr);
                if let Some(http_addr) = http_addr {
                    if let Ok(devices) = BridgeTransportClient::new(http_addr, String::new())
                        .list_devices()
                    {
                        if let Some(device) = devices.iter().find(|d| d.id == device_id) {
                            let active_id = format!("bridge:{}:{}", bridge_id, device.id);
                            *device_to_set = Some(device.name.clone());
                            return Ok((Some(bridge_id), Some(active_id)));
                        }
                        if let Some(device) = devices.iter().find(|d| d.name == device_id) {
                            let active_id = format!("bridge:{}:{}", bridge_id, device.id);
                            *device_to_set = Some(device.name.clone());
                            return Ok((Some(bridge_id), Some(active_id)));
                        }
                    }
                }
                let active_id = format!("bridge:{}:{}", bridge_id, device_id);
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
                    match BridgeTransportClient::new(bridge.http_addr, String::new())
                        .list_devices()
                    {
                        Ok(devices) if !devices.is_empty() => {
                            let device = devices[0].clone();
                            let active_id = format!("bridge:{}:{}", bridge.id, device.id);
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

/// Install Ctrl+C handler to stop playback cleanly.
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
