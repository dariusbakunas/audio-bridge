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
use anyhow::{Context as AnyhowContext, Result};
use rustls::ServerConfig as RustlsConfig;
use crossbeam_channel::unbounded;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::api;
use crate::bridge_transport::BridgeTransportClient;
use crate::bridge_device_streams::{spawn_bridge_device_streams_for_config, spawn_bridge_status_streams_for_config};
use crate::bridge_manager::parse_output_id;
use crate::config;
use crate::cover_art::CoverArtFetcher;
use crate::discovery::{
    spawn_cast_mdns_discovery,
    spawn_discovered_health_watcher,
    spawn_mdns_discovery,
};
use crate::metadata_db::MetadataDb;
use crate::metadata_service::MetadataService;
use crate::musicbrainz::{MusicBrainzClient, spawn_enrichment_loop};
use crate::events::LogBus;
use crate::state::MetadataWake;
use crate::openapi;
use crate::state::{AppState, BridgeProviderState, BridgeState, CastProviderState, LocalProviderState, PlayerStatus, QueueState};

/// Build server state and start the Actix HTTP server.
pub(crate) async fn run(args: crate::Args, log_bus: std::sync::Arc<LogBus>) -> Result<()> {
    let (cfg, cfg_path) = load_config(args.config.as_ref())?;
    let bind = resolve_bind(args.bind, &cfg)?;
    let tls_config = resolve_tls_config(&args, &cfg)?;
    let public_base_url = config::public_base_url_from_config(&cfg, bind, tls_config.is_some())?;
    let media_dir = resolve_media_dir(args.media_dir, &cfg)?;
    tracing::info!(
        bind = %bind,
        public_base_url = %public_base_url,
        media_dir = %media_dir.display(),
        "starting audio-hub-server"
    );
    let web_ui_dist = locate_web_ui_dist();
    if let Some(dist) = web_ui_dist.as_ref() {
        tracing::info!(path = %dist.display(), "web ui static assets enabled");
    } else {
        tracing::info!("web ui static assets disabled (web-ui/dist not found)");
    }
    let events = crate::events::EventBus::new();
    let metadata_wake = MetadataWake::new();
    let (metadata_db, library) =
        init_metadata_db_and_library(&media_dir, events.clone(), metadata_wake.clone())?;
    let musicbrainz = init_musicbrainz(&cfg)?;
    let bridges = config::bridges_from_config(&cfg)?;
    tracing::info!(
        count = bridges.len(),
        ids = ?bridges.iter().map(|b| b.id.clone()).collect::<Vec<_>>(),
        "loaded bridges from config"
    );

    let mut device_to_set: Option<String> = None;
    let (active_bridge_id, active_output_id) =
        resolve_active_output(&cfg, &bridges, &mut device_to_set).await?;
    let active_http_addr = active_bridge_id.as_ref().and_then(|bridge_id| {
        bridges
            .iter()
            .find(|b| b.id == *bridge_id)
            .map(|b| b.http_addr)
    });

    let output_settings_state = crate::state::OutputSettingsState::from_config(cfg.outputs.as_ref());
    let active_exclusive = active_output_id
        .as_deref()
        .map(|id| output_settings_state.is_exclusive(id))
        .unwrap_or(false);
    apply_active_bridge_device(
        device_to_set,
        active_http_addr,
        &public_base_url,
        active_exclusive,
    )
    .await;
    let bridge_state = build_bridge_state(bridges, active_bridge_id, active_output_id, public_base_url);
    let playback_manager = build_playback_manager(bridge_state.player.clone(), events.clone());
    let (local_state, device_selection) = build_local_state(&cfg);
    let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
    let cast_state = Arc::new(CastProviderState::new());
    let output_settings = Arc::new(Mutex::new(output_settings_state));
    let state = web::Data::new(AppState::new(
        library,
        metadata_db,
        musicbrainz,
        metadata_wake.clone(),
        bridge_state,
        local_state,
        browser_state,
        cast_state,
        playback_manager,
        device_selection,
        events,
        log_bus,
        output_settings,
        cfg_path,
    ));
    spawn_library_watcher(state.clone());
    if let Some(client) = state.metadata.musicbrainz.as_ref() {
        spawn_enrichment_loop(
            state.metadata.db.clone(),
            client.clone(),
            state.events.clone(),
            metadata_wake.clone(),
        );
        CoverArtFetcher::new(
            state.metadata.db.clone(),
            state.library.read().unwrap().root().to_path_buf(),
            client.user_agent().to_string(),
            state.events.clone(),
            metadata_wake.clone(),
        )
        .spawn();
    }
    setup_shutdown(state.providers.bridge.player.clone());
    spawn_mdns_discovery(state.clone());
    spawn_discovered_health_watcher(state.clone());
    spawn_cast_mdns_discovery(state.clone());
    spawn_bridge_device_streams_for_config(state.clone());
    spawn_bridge_status_streams_for_config(state.clone());
    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allowed_origin("http://localhost:5173")
            .allowed_origin("http://127.0.0.1:5173")
            .allowed_origin("tauri://localhost")
            .allowed_origin("http://tauri.localhost")
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
            .service(api::rescan_track)
            .service(api::play_track)
            .service(api::play_album)
            .service(api::pause_toggle)
            .service(api::stop)
            .service(api::seek)
            .service(api::stream_track)
            .service(api::stream_track_id)
            .service(api::transcode_track)
            .service(api::queue_list)
            .service(api::queue_add)
            .service(api::queue_add_next)
            .service(api::queue_remove)
            .service(api::queue_play_from)
            .service(api::queue_clear)
            .service(api::queue_next)
            .service(api::queue_previous)
            .service(api::queue_stream)
            .service(api::artists_list)
            .service(api::albums_list)
            .service(api::tracks_list)
            .service(api::tracks_resolve)
            .service(api::tracks_metadata)
            .service(api::tracks_metadata_fields)
            .service(api::tracks_metadata_update)
            .service(api::tracks_analysis)
            .service(api::albums_metadata)
            .service(api::albums_metadata_update)
            .service(api::artist_profile)
            .service(api::artist_profile_update)
            .service(api::album_profile)
            .service(api::album_profile_update)
            .service(api::artist_image_set)
            .service(api::artist_image_clear)
            .service(api::album_image_set)
            .service(api::album_image_clear)
            .service(api::media_asset)
            .service(api::musicbrainz_match_search)
            .service(api::musicbrainz_match_apply)
            .service(api::art_for_track)
            .service(api::track_cover)
            .service(api::album_cover)
            .service(api::logs_clear)
            .service(api::local_playback_register)
            .service(api::local_playback_play)
            .service(api::local_playback_sessions)
            .service(api::sessions_create)
            .service(api::sessions_list)
            .service(api::sessions_heartbeat)
            .service(api::browser_ws)
            .service(api::health::health)
            .service(api::status_for_output)
            .service(api::status_stream)
            .service(api::active_status_stream)
            .service(api::providers_list)
            .service(api::provider_outputs_list)
            .service(api::provider_refresh)
            .service(api::outputs_list)
            .service(api::outputs_stream)
            .service(api::metadata_stream)
            .service(api::albums_stream)
            .service(api::logs_stream)
            .service(api::outputs_select)
            .service(api::outputs_settings)
            .service(api::outputs_settings_update);

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
    });

    if let Some(tls_config) = tls_config {
        server
            .bind_rustls_0_22(bind, tls_config)?
            .run()
            .await?;
    } else {
        server
            .bind(bind)?
            .run()
            .await?;
    }

    Ok(())
}

fn spawn_library_watcher(state: web::Data<AppState>) {
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = state.metadata_service();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(tx) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::warn!(error = %err, "metadata watcher init failed");
                return;
            }
        };
        if let Err(err) = watcher.watch(&root, RecursiveMode::Recursive) {
            tracing::warn!(error = %err, "metadata watcher setup failed");
            return;
        }
        loop {
            let first = match rx.recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            let mut events = vec![first];
            while let Ok(event) = rx.recv_timeout(Duration::from_millis(750)) {
                events.push(event);
            }
            for event in events {
                let event = match event {
                    Ok(event) => event,
                    Err(err) => {
                        tracing::warn!(error = %err, "metadata watcher event error");
                        continue;
                    }
                };
                match event.kind {
                    EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        if event.paths.len() >= 2 {
                            let from = &event.paths[0];
                            let to = &event.paths[1];
                            let _ = metadata_service.remove_track_by_path(&state.library, from);
                            if !to.is_dir() {
                                let _ = metadata_service.rescan_track(&state.library, to);
                            }
                        } else {
                            for path in event.paths {
                                if path.is_dir() {
                                    continue;
                                }
                                let _ = metadata_service.rescan_track(&state.library, &path);
                            }
                        }
                    }
                    EventKind::Create(_)
                    | EventKind::Modify(_)
                    | EventKind::Access(_)
                    | EventKind::Other => {
                        for path in event.paths {
                            if path.is_dir() {
                                continue;
                            }
                            if let Err(response) = metadata_service.rescan_track(&state.library, &path) {
                                let status = response.status();
                                if status != actix_web::http::StatusCode::NOT_FOUND
                                    && status != actix_web::http::StatusCode::BAD_REQUEST
                                {
                                    tracing::warn!(
                                        status = %status,
                                        path = %path.display(),
                                        "metadata watcher rescan failed"
                                    );
                                }
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in event.paths {
                            if let Err(response) =
                                metadata_service.remove_track_by_path(&state.library, &path)
                            {
                                let status = response.status();
                                if status != actix_web::http::StatusCode::NOT_FOUND
                                    && status != actix_web::http::StatusCode::BAD_REQUEST
                                {
                                    tracing::warn!(
                                        status = %status,
                                        path = %path.display(),
                                        "metadata watcher remove failed"
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    });
}

/// Return true when the request path should be logged.
fn should_log_path(path: &str) -> bool {
    if path == "/queue"
        || path == "/stream"
        || path == "/queue/stream"
        || path == "/logs/stream"
        || path == "/logs/clear"
        || path == "/local-playback/sessions"
        || path == "/health"
        || path.ends_with("/status/stream")
        || path.starts_with("/stream/track/")
    {
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
fn load_config(path: Option<&PathBuf>) -> Result<(config::ServerConfig, Option<PathBuf>)> {
    match path {
        Some(path) => {
            let cfg = config::ServerConfig::load(path)?;
            Ok((cfg, Some(path.to_path_buf())))
        }
        None => {
            let auto_path = std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|dir| dir.join("config.toml")));
            if let Some(path) = auto_path.as_ref() {
                if path.exists() {
                    let cfg = config::ServerConfig::load(path)?;
                    Ok((cfg, Some(path.to_path_buf())))
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

fn resolve_tls_config(args: &crate::Args, cfg: &config::ServerConfig) -> Result<Option<RustlsConfig>> {
    let cert_path = args
        .tls_cert
        .as_ref()
        .map(|p| p.to_path_buf())
        .or_else(|| cfg.tls_cert.as_ref().map(PathBuf::from));
    let key_path = args
        .tls_key
        .as_ref()
        .map(|p| p.to_path_buf())
        .or_else(|| cfg.tls_key.as_ref().map(PathBuf::from));

    let (Some(cert_path), Some(key_path)) = (cert_path, key_path) else {
        return Ok(None);
    };

    let cert_file = std::fs::File::open(&cert_path)
        .with_context(|| format!("open tls cert {:?}", cert_path))?;
    let mut cert_reader = std::io::BufReader::new(cert_file);
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("read tls cert {:?}", cert_path))?;
    if certs.is_empty() {
        return Err(anyhow::anyhow!("tls cert is empty: {:?}", cert_path));
    }

    let key_file = std::fs::File::open(&key_path)
        .with_context(|| format!("open tls key {:?}", key_path))?;
    let mut key_reader = std::io::BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .with_context(|| format!("read tls key {:?}", key_path))?
        .ok_or_else(|| anyhow::anyhow!("tls key is empty: {:?}", key_path))?;

    let config = RustlsConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| anyhow::anyhow!("invalid tls config: {err}"))?;
    Ok(Some(config))
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

fn init_musicbrainz(cfg: &config::ServerConfig) -> Result<Option<Arc<MusicBrainzClient>>> {
    let musicbrainz = match cfg.musicbrainz.as_ref() {
        Some(cfg) => MusicBrainzClient::new(cfg)?,
        None => None,
    }
    .map(Arc::new);
    if let Some(client) = musicbrainz.as_ref() {
        tracing::info!(user_agent = client.user_agent(), "musicbrainz enrichment enabled");
    } else {
        tracing::info!("musicbrainz enrichment disabled");
    }
    Ok(musicbrainz)
}

fn init_metadata_db_and_library(
    media_dir: &PathBuf,
    events: crate::events::EventBus,
    metadata_wake: MetadataWake,
) -> Result<(MetadataDb, crate::library::LibraryIndex)> {
    let metadata_db = MetadataDb::new(media_dir)?;
    let metadata_service = MetadataService::new(
        metadata_db.clone(),
        media_dir.clone(),
        events,
        metadata_wake,
    );
    let library = metadata_service.scan_library(false)?;
    metadata_service.ensure_album_markers();
    Ok((metadata_db, library))
}

async fn apply_active_bridge_device(
    device_to_set: Option<String>,
    active_http_addr: Option<std::net::SocketAddr>,
    public_base_url: &str,
    exclusive: bool,
) {
    if let (Some(device_name), Some(http_addr)) = (device_to_set, active_http_addr) {
        let _ = BridgeTransportClient::new_with_base(
            http_addr,
            public_base_url.to_string(),
            None,
        )
        .set_device(&device_name, Some(exclusive))
        .await;
    }
}

fn build_bridge_state(
    bridges: Vec<crate::config::BridgeConfigResolved>,
    active_bridge_id: Option<String>,
    active_output_id: Option<String>,
    public_base_url: String,
) -> Arc<BridgeProviderState> {
    let (cmd_tx, _cmd_rx) = unbounded();
    let bridge_online = Arc::new(AtomicBool::new(false));
    let bridges_state = Arc::new(Mutex::new(BridgeState {
        bridges,
        active_bridge_id,
        active_output_id,
    }));
    let discovered_bridges = Arc::new(Mutex::new(std::collections::HashMap::new()));
    Arc::new(BridgeProviderState::new(
        cmd_tx,
        bridges_state,
        bridge_online,
        discovered_bridges,
        public_base_url,
    ))
}

fn build_playback_manager(
    player: Arc<Mutex<crate::bridge::BridgePlayer>>,
    events: crate::events::EventBus,
) -> crate::playback_manager::PlaybackManager {
    let status = Arc::new(Mutex::new(PlayerStatus::default()));
    let queue = Arc::new(Mutex::new(QueueState::default()));
    let status_store = crate::status_store::StatusStore::new(status, events.clone());
    let queue_service = crate::queue_service::QueueService::new(queue, status_store.clone(), events);
    crate::playback_manager::PlaybackManager::new(player, status_store, queue_service)
}

fn build_local_state(
    cfg: &config::ServerConfig,
) -> (Arc<LocalProviderState>, crate::state::DeviceSelectionState) {
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
    (local_state, device_selection)
}

/// Resolve active output id from config and available bridges.
async fn resolve_active_output(
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
                    if let Ok(devices) = BridgeTransportClient::new(http_addr)
                        .list_devices()
                        .await
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
                    match BridgeTransportClient::new(bridge.http_addr)
                        .list_devices()
                        .await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_log_path_filters_noisy_paths() {
        assert!(!should_log_path("/queue"));
        assert!(!should_log_path("/queue/stream"));
        assert!(!should_log_path("/logs/stream"));
        assert!(!should_log_path("/outputs/bridge:test/status/stream"));
        assert!(!should_log_path("/outputs/bridge:test"));
        assert!(!should_log_path("/stream/track/31"));
        assert!(should_log_path("/artists"));
        assert!(should_log_path("/outputs/select"));
    }

    #[test]
    fn resolve_active_output_supports_local_override() {
        let system = actix_web::rt::System::new();
        let mut device_to_set = None;
        let cfg = config::ServerConfig {
            bind: None,
            media_dir: None,
            public_base_url: None,
            bridges: None,
            active_output: Some("local:default".to_string()),
            local_outputs: None,
            local_id: None,
            local_name: None,
            local_device: None,
            musicbrainz: None,
            tls_cert: None,
            tls_key: None,
            outputs: None,
        };
        let result = system.block_on(resolve_active_output(&cfg, &[], &mut device_to_set)).expect("resolve");
        assert_eq!(result.0, None);
        assert_eq!(result.1, Some("local:default".to_string()));
        assert_eq!(device_to_set, None);
    }

    #[test]
    fn resolve_active_output_defaults_to_none_without_bridges() {
        let system = actix_web::rt::System::new();
        let mut device_to_set = None;
        let cfg = config::ServerConfig {
            bind: None,
            media_dir: None,
            public_base_url: None,
            bridges: None,
            active_output: None,
            local_outputs: None,
            local_id: None,
            local_name: None,
            local_device: None,
            musicbrainz: None,
            tls_cert: None,
            tls_key: None,
            outputs: None,
        };
        let result = system.block_on(resolve_active_output(&cfg, &[], &mut device_to_set)).expect("resolve");
        assert_eq!(result, (None, None));
        assert_eq!(device_to_set, None);
    }
}
