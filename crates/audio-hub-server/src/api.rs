//! HTTP API handlers.
//!
//! Defines the Actix routes for library, playback, queue, and output control.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use actix_files::NamedFile;
use actix_web::{Error, get, post, web, HttpRequest, HttpResponse, Responder};
use actix_web::http::{header, StatusCode};
use actix_web::body::SizedStream;
use actix_web::web::Bytes;
use futures_util::stream::unfold;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use tokio_util::io::ReaderStream;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::time::{Duration, Interval, MissedTickBehavior};
use tokio::sync::broadcast::error::RecvError;

use crate::metadata_service::MetadataService;
use crate::musicbrainz::MusicBrainzMatch;
use crate::models::{
    AlbumListResponse,
    ArtistListResponse,
    LibraryResponse,
    MusicBrainzMatchApplyRequest,
    MusicBrainzMatchCandidate,
    MusicBrainzMatchKind,
    MusicBrainzMatchSearchRequest,
    MusicBrainzMatchSearchResponse,
    PlayRequest,
    QueueMode,
    QueueAddRequest,
    QueueRemoveRequest,
    QueuePlayFromRequest,
    QueueResponse,
    StatusResponse,
    OutputsResponse,
    OutputSelectRequest,
    ProvidersResponse,
    TrackResolveResponse,
    TrackMetadataResponse,
    TrackMetadataUpdateRequest,
    AlbumMetadataResponse,
    AlbumMetadataUpdateRequest,
    TrackListResponse,
};
use crate::events::{HubEvent, LogEvent};
use crate::state::AppState;
use crate::tag_writer::{write_track_tags, TrackTagUpdate};

/// Query parameters for library listing.
#[derive(Deserialize, ToSchema)]
pub struct LibraryQuery {
    /// Optional directory to list under the library root.
    pub dir: Option<String>,
}

/// Query parameters for stream requests.
#[derive(Deserialize, ToSchema)]
pub struct StreamQuery {
    /// Absolute path to the media file.
    pub path: String,
}

/// Seek request payload (milliseconds).
#[derive(Deserialize, ToSchema)]
pub struct SeekBody {
    /// Absolute seek position in milliseconds.
    pub ms: u64,
}

#[derive(Deserialize, ToSchema)]
pub struct ListQuery {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
pub struct AlbumListQuery {
    #[serde(default)]
    pub artist_id: Option<i64>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
pub struct TrackListQuery {
    #[serde(default)]
    pub album_id: Option<i64>,
    #[serde(default)]
    pub artist_id: Option<i64>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct LogsClearResponse {
    pub cleared_at_ms: i64,
}

#[utoipa::path(
    post,
    path = "/logs/clear",
    responses(
        (status = 200, description = "Log buffer cleared", body = LogsClearResponse)
    )
)]
#[post("/logs/clear")]
/// Clear the in-memory log buffer.
pub async fn logs_clear(state: web::Data<AppState>) -> impl Responder {
    state.log_bus.clear();
    let cleared_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    HttpResponse::Ok().json(LogsClearResponse { cleared_at_ms })
}

#[utoipa::path(
    get,
    path = "/library",
    params(
        ("dir" = Option<String>, Query, description = "Directory to list")
    ),
    responses(
        (status = 200, description = "Library entries", body = LibraryResponse)
    )
)]
#[get("/library")]
/// List library entries for the requested directory.
pub async fn list_library(state: web::Data<AppState>, query: web::Query<LibraryQuery>) -> impl Responder {
    let dir = query
        .dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.library.read().unwrap().root().to_path_buf());

    let dir = match state.output_controller.canonicalize_under_root(&state, &dir) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
    };

    let library = state.library.read().unwrap();
    let entries = match library.list_dir(&dir) {
        Some(entries) => entries.to_vec(),
        None => Vec::new(),
    };
    let resp = LibraryResponse {
        dir: dir.to_string_lossy().to_string(),
        entries,
    };
    HttpResponse::Ok().json(resp)
}

#[utoipa::path(
    get,
    path = "/stream",
    params(
        ("path" = String, Query, description = "Track path under the library root")
    ),
    responses(
        (status = 200, description = "Full file stream"),
        (status = 206, description = "Partial content"),
        (status = 404, description = "Not found"),
        (status = 416, description = "Invalid range")
    )
)]
#[get("/stream")]
/// Stream a track with HTTP range support.
pub async fn stream_track(
    state: web::Data<AppState>,
    req: HttpRequest,
    query: web::Query<StreamQuery>,
) -> impl Responder {
    let path = PathBuf::from(&query.path);
    let path = match state.output_controller.canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    let meta = match file.metadata().await {
        Ok(m) => m,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    let total_len = meta.len();

    let range_header = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let range = match range_header.and_then(|h| parse_single_range(h, total_len)) {
        Some(r) => Some(r),
        None if range_header.is_some() => {
            return HttpResponse::RangeNotSatisfiable()
                .insert_header((header::ACCEPT_RANGES, "bytes"))
                .finish();
        }
        None => None,
    };

    let (start, len, status_code) = if let Some((start, end)) = range {
        let len = end.saturating_sub(start).saturating_add(1);
        (start, len, StatusCode::PARTIAL_CONTENT)
    } else {
        (0, total_len, StatusCode::OK)
    };

    if start > 0 {
        if let Err(_) = file.seek(std::io::SeekFrom::Start(start)).await {
            return HttpResponse::InternalServerError().finish();
        }
    }

    let stream = ReaderStream::new(file.take(len));
    let body = SizedStream::new(len, stream);

    let mut resp = HttpResponse::build(status_code);
    resp.insert_header((header::ACCEPT_RANGES, "bytes"));
    if let Some((start, end)) = range {
        resp.insert_header((
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total_len}"),
        ));
    }
    resp.insert_header((header::CONTENT_LENGTH, len.to_string()));
    resp.body(body)
}

#[utoipa::path(
    post,
    path = "/library/rescan",
    responses(
        (status = 200, description = "Rescan started"),
        (status = 500, description = "Rescan failed")
    )
)]
#[post("/library/rescan")]
/// Trigger a full library rescan.
pub async fn rescan_library(state: web::Data<AppState>) -> impl Responder {
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        root.clone(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    tracing::info!(root = %root.display(), "rescan requested");
    match metadata_service.rescan_library(true) {
        Ok(new_index) => {
            *state.library.write().unwrap() = new_index;
            state.events.library_changed();
            state.metadata_wake.notify();
            HttpResponse::Ok().finish()
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("scan failed: {e:#}")),
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct RescanTrackRequest {
    pub path: String,
}

#[utoipa::path(
    post,
    path = "/library/rescan/track",
    request_body = RescanTrackRequest,
    responses(
        (status = 200, description = "Track rescan completed"),
        (status = 400, description = "Invalid path"),
        (status = 404, description = "Track not found")
    )
)]
#[post("/library/rescan/track")]
/// Rescan metadata for a single track.
pub async fn rescan_track(
    state: web::Data<AppState>,
    body: web::Json<RescanTrackRequest>,
) -> impl Responder {
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        root.clone(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let full_path = match MetadataService::resolve_track_path(&root, &body.path) {
        Ok(path) => path,
        Err(response) => return response,
    };
    if let Err(response) = metadata_service.rescan_track(&state.library, &full_path) {
        return response;
    }
    HttpResponse::Ok().finish()
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct ArtQuery {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct TrackResolveQuery {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct TrackMetadataQuery {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct AlbumMetadataQuery {
    pub album_id: i64,
}

#[utoipa::path(
    get,
    path = "/tracks/resolve",
    params(TrackResolveQuery),
    responses(
        (status = 200, description = "Resolved track metadata", body = TrackResolveResponse),
        (status = 404, description = "Track not found")
    )
)]
#[get("/tracks/resolve")]
/// Resolve a track path to album metadata.
pub async fn tracks_resolve(
    state: web::Data<AppState>,
    query: web::Query<TrackResolveQuery>,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    match metadata_service.album_id_for_track_path(&query.path) {
        Ok(Some(album_id)) => HttpResponse::Ok().json(TrackResolveResponse {
            album_id: Some(album_id),
        }),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(err) => HttpResponse::InternalServerError().body(err),
    }
}

#[utoipa::path(
    get,
    path = "/tracks/metadata",
    params(TrackMetadataQuery),
    responses(
        (status = 200, description = "Track metadata", body = TrackMetadataResponse),
        (status = 404, description = "Track not found")
    )
)]
#[get("/tracks/metadata")]
/// Read cached metadata for a track path.
pub async fn tracks_metadata(
    state: web::Data<AppState>,
    query: web::Query<TrackMetadataQuery>,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    match metadata_service.track_record_by_path(&query.path) {
        Ok(Some(record)) => HttpResponse::Ok().json(TrackMetadataResponse {
            path: record.path,
            title: record.title,
            artist: record.artist,
            album: record.album,
            album_artist: record.album_artist,
            year: record.year,
            track_number: record.track_number,
            disc_number: record.disc_number,
        }),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(err) => HttpResponse::InternalServerError().body(err),
    }
}

#[utoipa::path(
    post,
    path = "/tracks/metadata/update",
    request_body = TrackMetadataUpdateRequest,
    responses(
        (status = 200, description = "Track metadata updated"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Track not found")
    )
)]
#[post("/tracks/metadata/update")]
/// Write tag metadata into a track file.
pub async fn tracks_metadata_update(
    state: web::Data<AppState>,
    body: web::Json<TrackMetadataUpdateRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        root.clone(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let full_path = match MetadataService::resolve_track_path(&root, &request.path) {
        Ok(path) => path,
        Err(response) => return response,
    };

    let title = request.title.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let artist = request.artist.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let album = request.album.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let album_artist = request
        .album_artist
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let year = request.year.filter(|value| *value > 0);
    let track_number = request.track_number.filter(|value| *value > 0);
    let disc_number = request.disc_number.filter(|value| *value > 0);

    if title.is_none()
        && artist.is_none()
        && album.is_none()
        && album_artist.is_none()
        && year.is_none()
        && track_number.is_none()
        && disc_number.is_none()
    {
        return HttpResponse::BadRequest().body("no metadata fields provided");
    }

    if let Err(err) = write_track_tags(
        &full_path,
        TrackTagUpdate {
            title,
            artist,
            album,
            album_artist,
            year,
            track_number,
            disc_number,
        },
    ) {
        tracing::warn!(error = %err, path = %request.path, "track metadata update failed");
        return HttpResponse::InternalServerError().body(err.to_string());
    }

    if let Err(response) = metadata_service.rescan_track(&state.library, &full_path) {
        return response;
    }

    HttpResponse::Ok().finish()
}

#[utoipa::path(
    get,
    path = "/albums/metadata",
    params(AlbumMetadataQuery),
    responses(
        (status = 200, description = "Album metadata", body = AlbumMetadataResponse),
        (status = 404, description = "Album not found")
    )
)]
#[get("/albums/metadata")]
/// Read cached metadata for an album id.
pub async fn albums_metadata(
    state: web::Data<AppState>,
    query: web::Query<AlbumMetadataQuery>,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    match metadata_service.album_summary_by_id(query.album_id) {
        Ok(Some(album)) => HttpResponse::Ok().json(AlbumMetadataResponse {
            album_id: album.id,
            title: Some(album.title),
            album_artist: album.artist,
            year: album.year,
        }),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(err) => HttpResponse::InternalServerError().body(err),
    }
}

#[utoipa::path(
    post,
    path = "/albums/metadata/update",
    request_body = AlbumMetadataUpdateRequest,
    responses(
        (status = 200, description = "Album metadata updated"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Album not found")
    )
)]
#[post("/albums/metadata/update")]
/// Write album metadata into all tracks for an album.
pub async fn albums_metadata_update(
    state: web::Data<AppState>,
    body: web::Json<AlbumMetadataUpdateRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        root.clone(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let album = request.album.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let album_artist = request
        .album_artist
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let year = request.year.filter(|value| *value > 0);
    let track_artist = request
        .track_artist
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    if album.is_none()
        && album_artist.is_none()
        && year.is_none()
        && track_artist.is_none()
    {
        return HttpResponse::BadRequest().body("no metadata fields provided");
    }

    let paths = match metadata_service.list_track_paths_by_album_id(request.album_id) {
        Ok(paths) => paths,
        Err(err) => return HttpResponse::InternalServerError().body(err),
    };
    if paths.is_empty() {
        return HttpResponse::NotFound().finish();
    }

    for path in paths {
        let full_path = match MetadataService::resolve_track_path(&root, &path) {
            Ok(path) => path,
            Err(response) => return response,
        };
        if let Err(err) = write_track_tags(
            &full_path,
            TrackTagUpdate {
                title: None,
                artist: track_artist,
                album,
                album_artist,
                year,
                track_number: None,
                disc_number: None,
            },
        ) {
            tracing::warn!(
                error = %err,
                path = %path,
                album_id = request.album_id,
                "album metadata update failed"
            );
            return HttpResponse::InternalServerError().body(err.to_string());
        }
        if let Err(response) = metadata_service.rescan_track(&state.library, &full_path) {
            return response;
        }
    }

    if album.is_some() || album_artist.is_some() || year.is_some() {
        match metadata_service.update_album_metadata(request.album_id, album, album_artist, year) {
            Ok(true) => {}
            Ok(false) => return HttpResponse::NotFound().finish(),
            Err(err) => return HttpResponse::InternalServerError().body(err),
        }
    }

    HttpResponse::Ok().finish()
}

#[utoipa::path(
    get,
    path = "/art",
    params(ArtQuery),
    responses(
        (status = 200, description = "Cover art image"),
        (status = 404, description = "Cover art not found")
    )
)]
#[get("/art")]
pub async fn art_for_track(
    state: web::Data<AppState>,
    query: web::Query<ArtQuery>,
    req: HttpRequest,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let cover_rel = match metadata_service.cover_path_for_track(&query.path) {
        Ok(Some(path)) => path,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err),
    };
    serve_cover_art(&state, &cover_rel, &req)
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct CoverPath {
    pub id: i64,
}

#[utoipa::path(
    get,
    path = "/tracks/{id}/cover",
    params(CoverPath),
    responses(
        (status = 200, description = "Cover art image"),
        (status = 404, description = "Cover art not found")
    )
)]
#[get("/tracks/{id}/cover")]
pub async fn track_cover(
    state: web::Data<AppState>,
    path: web::Path<CoverPath>,
    req: HttpRequest,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let cover_rel = match metadata_service.cover_path_for_track_id(path.id) {
        Ok(Some(path)) => path,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err),
    };
    serve_cover_art(&state, &cover_rel, &req)
}

#[utoipa::path(
    get,
    path = "/albums/{id}/cover",
    params(CoverPath),
    responses(
        (status = 200, description = "Cover art image"),
        (status = 404, description = "Cover art not found")
    )
)]
#[get("/albums/{id}/cover")]
pub async fn album_cover(
    state: web::Data<AppState>,
    path: web::Path<CoverPath>,
    req: HttpRequest,
) -> impl Responder {
    let metadata_service = MetadataService::new(
        state.metadata_db.clone(),
        state.library.read().unwrap().root().to_path_buf(),
        state.events.clone(),
        state.metadata_wake.clone(),
    );
    let cover_rel = match metadata_service.cover_path_for_album_id(path.id) {
        Ok(Some(path)) => path,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err),
    };
    serve_cover_art(&state, &cover_rel, &req)
}

fn serve_cover_art(state: &AppState, cover_rel: &str, req: &HttpRequest) -> HttpResponse {
    let root = state.library.read().unwrap().root().to_path_buf();
    let art_root = root.join(".audio-hub").join("art");
    let full_path = root.join(cover_rel);
    let full_path = match full_path.canonicalize() {
        Ok(path) => path,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    if !full_path.starts_with(&art_root) {
        return HttpResponse::Forbidden().finish();
    }
    match NamedFile::open(full_path) {
        Ok(file) => file.into_response(req),
        Err(_) => HttpResponse::NotFound().finish(),
    }
}

#[utoipa::path(
    post,
    path = "/play",
    request_body = PlayRequest,
    responses(
        (status = 200, description = "Playback started"),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/play")]
/// Start playback for the requested track.
pub async fn play_track(state: web::Data<AppState>, body: web::Json<PlayRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match state.output_controller.canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
    };

    let mode = body.queue_mode.clone().unwrap_or(QueueMode::Keep);
    tracing::info!(path = %path.display(), "play request");
    let output_id = match state
        .output_controller
        .play_request(&state, path.clone(), mode, body.output_id.as_deref())
        .await
    {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };
    tracing::info!(output_id = %output_id, "play dispatched");
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/pause",
    responses(
        (status = 200, description = "Pause toggled"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/pause")]
/// Toggle pause/resume.
pub async fn pause_toggle(state: web::Data<AppState>) -> impl Responder {
    tracing::info!("pause toggle request");
    match state.output_controller.pause_toggle(&state).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/stop",
    responses(
        (status = 200, description = "Playback stopped"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/stop")]
/// Stop playback.
pub async fn stop(state: web::Data<AppState>) -> impl Responder {
    tracing::info!("stop request");
    match state.output_controller.stop(&state).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/seek",
    request_body = SeekBody,
    responses(
        (status = 200, description = "Seek requested"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/seek")]
/// Seek to an absolute position (milliseconds).
pub async fn seek(state: web::Data<AppState>, body: web::Json<SeekBody>) -> impl Responder {
    let ms = body.ms;
    match state.output_controller.seek(&state, ms).await {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/queue",
    responses(
        (status = 200, description = "Queue contents", body = QueueResponse)
    )
)]
#[get("/queue")]
/// Return the current queue.
pub async fn queue_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(state.output_controller.queue_list(&state))
}

#[utoipa::path(
    post,
    path = "/queue",
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated")
    )
)]
#[post("/queue")]
/// Add paths to the queue.
pub async fn queue_add(state: web::Data<AppState>, body: web::Json<QueueAddRequest>) -> impl Responder {
    let added = state
        .output_controller
        .queue_add_paths(&state, body.paths.clone());
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/queue/next/add",
    request_body = QueueAddRequest,
    responses(
        (status = 200, description = "Queue updated")
    )
)]
#[post("/queue/next/add")]
/// Insert paths at the front of the queue.
pub async fn queue_add_next(
    state: web::Data<AppState>,
    body: web::Json<QueueAddRequest>,
) -> impl Responder {
    let added = state
        .output_controller
        .queue_add_next_paths(&state, body.paths.clone());
    HttpResponse::Ok().body(format!("added {added}"))
}

#[utoipa::path(
    post,
    path = "/queue/remove",
    request_body = QueueRemoveRequest,
    responses(
        (status = 200, description = "Queue updated"),
        (status = 400, description = "Bad request")
    )
)]
#[post("/queue/remove")]
/// Remove a path from the queue.
pub async fn queue_remove(state: web::Data<AppState>, body: web::Json<QueueRemoveRequest>) -> impl Responder {
    match state
        .output_controller
        .queue_remove_path(&state, &body.path)
    {
        Ok(_) => HttpResponse::Ok().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/play_from",
    request_body = QueuePlayFromRequest,
    responses(
        (status = 200, description = "Playback started"),
        (status = 404, description = "Item not found"),
        (status = 500, description = "Player offline")
    )
)]
#[post("/queue/play_from")]
/// Play a queued item and drop items ahead of it.
pub async fn queue_play_from(
    state: web::Data<AppState>,
    body: web::Json<QueuePlayFromRequest>,
) -> impl Responder {
    match state
        .output_controller
        .queue_play_from(&state, &body.path)
        .await
    {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NotFound().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/clear",
    responses(
        (status = 200, description = "Queue cleared")
    )
)]
#[post("/queue/clear")]
/// Clear the queue.
pub async fn queue_clear(state: web::Data<AppState>) -> impl Responder {
    state.output_controller.queue_clear(&state);
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    get,
    path = "/artists",
    params(
        ("search" = Option<String>, Query, description = "Search term"),
        ("limit" = Option<i64>, Query, description = "Max rows"),
        ("offset" = Option<i64>, Query, description = "Offset rows")
    ),
    responses(
        (status = 200, description = "Artist list", body = ArtistListResponse)
    )
)]
#[get("/artists")]
/// List artists from the metadata database.
pub async fn artists_list(state: web::Data<AppState>, query: web::Query<ListQuery>) -> impl Responder {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);
    match state
        .metadata_db
        .list_artists(query.search.as_deref(), limit, offset)
    {
        Ok(items) => HttpResponse::Ok().json(ArtistListResponse { items }),
        Err(err) => {
            tracing::warn!(error = %err, "artists list failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[utoipa::path(
    get,
    path = "/albums",
    params(
        ("artist_id" = Option<i64>, Query, description = "Artist id"),
        ("search" = Option<String>, Query, description = "Search term"),
        ("limit" = Option<i64>, Query, description = "Max rows"),
        ("offset" = Option<i64>, Query, description = "Offset rows")
    ),
    responses(
        (status = 200, description = "Album list", body = AlbumListResponse)
    )
)]
#[get("/albums")]
/// List albums from the metadata database.
pub async fn albums_list(
    state: web::Data<AppState>,
    query: web::Query<AlbumListQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);
    match state.metadata_db.list_albums(
        query.artist_id,
        query.search.as_deref(),
        limit,
        offset,
    ) {
        Ok(items) => HttpResponse::Ok().json(AlbumListResponse { items }),
        Err(err) => {
            tracing::warn!(error = %err, "albums list failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[utoipa::path(
    get,
    path = "/tracks",
    params(
        ("album_id" = Option<i64>, Query, description = "Album id"),
        ("artist_id" = Option<i64>, Query, description = "Artist id"),
        ("search" = Option<String>, Query, description = "Search term"),
        ("limit" = Option<i64>, Query, description = "Max rows"),
        ("offset" = Option<i64>, Query, description = "Offset rows")
    ),
    responses(
        (status = 200, description = "Track list", body = TrackListResponse)
    )
)]
#[get("/tracks")]
/// List tracks from the metadata database.
pub async fn tracks_list(
    state: web::Data<AppState>,
    query: web::Query<TrackListQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);
    match state.metadata_db.list_tracks(
        query.album_id,
        query.artist_id,
        query.search.as_deref(),
        limit,
        offset,
    ) {
        Ok(items) => HttpResponse::Ok().json(TrackListResponse { items }),
        Err(err) => {
            tracing::warn!(error = %err, "tracks list failed");
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[utoipa::path(
    post,
    path = "/metadata/match/search",
    request_body = MusicBrainzMatchSearchRequest,
    responses(
        (status = 200, description = "MusicBrainz search results", body = MusicBrainzMatchSearchResponse),
        (status = 400, description = "Bad request")
    )
)]
#[post("/metadata/match/search")]
/// Search MusicBrainz to manually match a track or album.
pub async fn musicbrainz_match_search(
    state: web::Data<AppState>,
    body: web::Json<MusicBrainzMatchSearchRequest>,
) -> impl Responder {
    let Some(client) = state.musicbrainz.as_ref() else {
        return HttpResponse::BadRequest().body("musicbrainz is disabled");
    };
    let title = body.title.trim();
    let artist = body.artist.trim();
    if title.is_empty() || artist.is_empty() {
        return HttpResponse::BadRequest().body("title and artist are required");
    }
    let limit = body.limit.unwrap_or(10).clamp(1, 25);
    let results = match body.kind {
        MusicBrainzMatchKind::Track => {
            match client.search_recordings(title, artist, body.album.as_deref(), limit) {
                Ok(items) => items
                    .into_iter()
                    .map(|item| MusicBrainzMatchCandidate {
                        recording_mbid: Some(item.recording_mbid),
                        release_mbid: item.release_mbid,
                        artist_mbid: item.artist_mbid,
                        title: item.title,
                        artist: item.artist_name.unwrap_or_else(|| artist.to_string()),
                        release_title: item.release_title,
                        score: item.score,
                        year: item.year,
                    })
                    .collect::<Vec<_>>(),
                Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
            }
        }
        MusicBrainzMatchKind::Album => {
            match client.search_releases(title, artist, limit) {
                Ok(items) => items
                    .into_iter()
                    .map(|item| MusicBrainzMatchCandidate {
                        recording_mbid: None,
                        release_mbid: Some(item.release_mbid),
                        artist_mbid: item.artist_mbid,
                        title: item.title,
                        artist: item.artist_name.unwrap_or_else(|| artist.to_string()),
                        release_title: None,
                        score: item.score,
                        year: item.year,
                    })
                    .collect::<Vec<_>>(),
                Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
            }
        }
    };
    HttpResponse::Ok().json(MusicBrainzMatchSearchResponse { items: results })
}

#[utoipa::path(
    post,
    path = "/metadata/match/apply",
    request_body = MusicBrainzMatchApplyRequest,
    responses(
        (status = 200, description = "MusicBrainz match applied"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Target not found")
    )
)]
#[post("/metadata/match/apply")]
/// Apply a MusicBrainz match to a track or album.
pub async fn musicbrainz_match_apply(
    state: web::Data<AppState>,
    body: web::Json<MusicBrainzMatchApplyRequest>,
) -> impl Responder {
    let Some(_) = state.musicbrainz.as_ref() else {
        return HttpResponse::BadRequest().body("musicbrainz is disabled");
    };
    match body.into_inner() {
        MusicBrainzMatchApplyRequest::Track {
            path,
            recording_mbid,
            artist_mbid,
            album_mbid,
            release_year,
            override_existing,
        } => {
            tracing::info!(
                path = %path,
                recording_mbid = %recording_mbid,
                artist_mbid = ?artist_mbid,
                album_mbid = ?album_mbid,
                release_year = ?release_year,
                override_existing = ?override_existing,
                "manual musicbrainz match apply (track)"
            );
            let record = match state.metadata_db.track_record_by_path(&path) {
                Ok(Some(record)) => record,
                Ok(None) => return HttpResponse::NotFound().finish(),
                Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
            };
            let mb = MusicBrainzMatch {
                recording_mbid: Some(recording_mbid),
                artist_mbid,
                artist_name: None,
                artist_sort_name: None,
                album_mbid,
                album_title: None,
                release_year,
                release_candidates: Vec::new(),
            };
            let override_existing = override_existing.unwrap_or(true);
            if let Err(err) = state
                .metadata_db
                .apply_musicbrainz_with_override(&record, &mb, override_existing)
            {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
            tracing::info!(
                path = %record.path,
                album = ?record.album,
                "manual musicbrainz match applied (track)"
            );
            state.events.metadata_event(crate::events::MetadataEvent::MusicBrainzLookupSuccess {
                path: record.path.clone(),
                recording_mbid: mb.recording_mbid.clone(),
                artist_mbid: mb.artist_mbid.clone(),
                album_mbid: mb.album_mbid.clone(),
            });
            state.metadata_wake.notify();
        }
        MusicBrainzMatchApplyRequest::Album {
            album_id,
            album_mbid,
            artist_mbid,
            release_year,
            override_existing,
        } => {
            tracing::info!(
                album_id,
                album_mbid = %album_mbid,
                artist_mbid = ?artist_mbid,
                release_year = ?release_year,
                override_existing = ?override_existing,
                "manual musicbrainz match apply (album)"
            );
            let mb = MusicBrainzMatch {
                recording_mbid: None,
                artist_mbid,
                artist_name: None,
                artist_sort_name: None,
                album_mbid: Some(album_mbid),
                album_title: None,
                release_year,
                release_candidates: Vec::new(),
            };
            let override_existing = override_existing.unwrap_or(true);
            if let Err(err) = state
                .metadata_db
                .apply_album_musicbrainz(album_id, &mb, override_existing)
            {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
            tracing::info!(album_id, "manual musicbrainz match applied (album)");
            state.metadata_wake.notify();
        }
    }
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/queue/next",
    responses(
        (status = 200, description = "Advanced to next"),
        (status = 204, description = "End of queue")
    )
)]
#[post("/queue/next")]
/// Skip to the next queued track.
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    tracing::debug!("queue next request");
    match state.output_controller.queue_next(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/queue/previous",
    responses(
        (status = 200, description = "Playback started"),
        (status = 204, description = "No previous track")
    )
)]
#[post("/queue/previous")]
/// Skip to the previously played track.
pub async fn queue_previous(state: web::Data<AppState>) -> impl Responder {
    tracing::debug!("queue previous request");
    match state.output_controller.queue_previous(&state).await {
        Ok(true) => HttpResponse::Ok().finish(),
        Ok(false) => HttpResponse::NoContent().finish(),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/outputs/{id}/status",
    params(
        ("id" = String, Path, description = "Output id")
    ),
    responses(
        (status = 200, description = "Playback status for output", body = StatusResponse),
        (status = 400, description = "Unknown or inactive output")
    )
)]
#[get("/outputs/{id}/status")]
/// Return playback status for a specific output.
pub async fn status_for_output(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let output_id = id.into_inner();
    tracing::debug!(output_id = %output_id, "status for output request");
    match state.output_controller.status_for_output(&state, &output_id).await {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}

struct StatusStreamState {
    state: web::Data<AppState>,
    output_id: String,
    receiver: tokio::sync::broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_status: Option<String>,
    last_ping: Instant,
}

fn sse_event(event: &str, data: &str) -> Bytes {
    let mut payload = String::new();
    payload.push_str("event: ");
    payload.push_str(event);
    payload.push('\n');
    for line in data.lines() {
        payload.push_str("data: ");
        payload.push_str(line);
        payload.push('\n');
    }
    payload.push('\n');
    Bytes::from(payload)
}

fn normalize_outputs_response(mut resp: OutputsResponse) -> OutputsResponse {
    if let Some(active_id) = resp.active_id.as_deref() {
        if !resp.outputs.iter().any(|o| o.id == active_id) {
            resp.active_id = None;
        }
    }
    resp
}

struct QueueStreamState {
    state: web::Data<AppState>,
    receiver: tokio::sync::broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_queue: Option<String>,
    last_ping: Instant,
}

struct OutputsStreamState {
    state: web::Data<AppState>,
    receiver: tokio::sync::broadcast::Receiver<HubEvent>,
    interval: Interval,
    pending: VecDeque<Bytes>,
    last_outputs: Option<String>,
    last_ping: Instant,
}

struct MetadataStreamState {
    receiver: tokio::sync::broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

struct LogsStreamState {
    receiver: tokio::sync::broadcast::Receiver<LogEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

struct AlbumsStreamState {
    receiver: tokio::sync::broadcast::Receiver<HubEvent>,
    pending: VecDeque<Bytes>,
    last_ping: Instant,
}

#[utoipa::path(
    get,
    path = "/outputs/{id}/status/stream",
    params(
        ("id" = String, Path, description = "Output id")
    ),
    responses(
        (status = 200, description = "Status event stream")
    )
)]
#[get("/outputs/{id}/status/stream")]
/// Stream status updates via server-sent events.
pub async fn status_stream(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    let output_id = id.into_inner();
    let initial = match state.output_controller.status_for_output(&state, &output_id).await {
        Ok(resp) => resp,
        Err(err) => return err.into_response(),
    };
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("status", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        StatusStreamState {
            state: state.clone(),
            output_id,
            receiver,
            interval,
            pending,
            last_status: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                let mut refresh = false;
                tokio::select! {
                    _ = ctx.interval.tick() => {}
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(HubEvent::StatusChanged) => refresh = true,
                            Ok(HubEvent::QueueChanged) => {}
                            Ok(HubEvent::OutputsChanged) => {}
                            Ok(HubEvent::Metadata(_)) => {}
                            Ok(HubEvent::LibraryChanged) => {}
                            Err(RecvError::Lagged(_)) => refresh = true,
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if refresh {
                    if let Ok(status) = ctx
                        .state
                        .output_controller
                        .status_for_output(&ctx.state, &ctx.output_id)
                        .await
                    {
                        let json = serde_json::to_string(&status)
                            .unwrap_or_else(|_| "null".to_string());
                        if ctx.last_status.as_deref() != Some(json.as_str()) {
                            ctx.last_status = Some(json.clone());
                            ctx.pending.push_back(sse_event("status", &json));
                        }
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/queue/stream",
    responses(
        (status = 200, description = "Queue event stream")
    )
)]
#[get("/queue/stream")]
/// Stream queue updates via server-sent events.
pub async fn queue_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = state.output_controller.queue_list(&state);
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("queue", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        QueueStreamState {
            state: state.clone(),
            receiver,
            interval,
            pending,
            last_queue: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                tokio::select! {
                    _ = ctx.interval.tick() => {}
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(HubEvent::QueueChanged) => {
                                let queue = ctx.state.output_controller.queue_list(&ctx.state);
                                let json = serde_json::to_string(&queue).unwrap_or_else(|_| "null".to_string());
                                if ctx.last_queue.as_deref() != Some(json.as_str()) {
                                    ctx.last_queue = Some(json.clone());
                                    ctx.pending.push_back(sse_event("queue", &json));
                                }
                            }
                            Ok(HubEvent::StatusChanged) => {}
                            Ok(HubEvent::OutputsChanged) => {}
                            Ok(HubEvent::Metadata(_)) => {}
                            Ok(HubEvent::LibraryChanged) => {}
                            Err(RecvError::Lagged(_)) => {
                                let queue = ctx.state.output_controller.queue_list(&ctx.state);
                                let json = serde_json::to_string(&queue).unwrap_or_else(|_| "null".to_string());
                                ctx.last_queue = Some(json.clone());
                                ctx.pending.push_back(sse_event("queue", &json));
                            }
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/outputs/stream",
    responses(
        (status = 200, description = "Outputs event stream")
    )
)]
#[get("/outputs/stream")]
/// Stream output updates via server-sent events.
pub async fn outputs_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = normalize_outputs_response(state.output_controller.list_outputs(&state));
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "null".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("outputs", &initial_json));

    let mut interval = tokio::time::interval(Duration::from_millis(2000));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let receiver = state.events.subscribe();

    let stream = unfold(
        OutputsStreamState {
            state: state.clone(),
            receiver,
            interval,
            pending,
            last_outputs: Some(initial_json),
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                let mut refresh = false;
                tokio::select! {
                    _ = ctx.interval.tick() => {}
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(HubEvent::OutputsChanged) => refresh = true,
                            Ok(HubEvent::StatusChanged) => {}
                            Ok(HubEvent::QueueChanged) => {}
                            Ok(HubEvent::Metadata(_)) => {}
                            Ok(HubEvent::LibraryChanged) => {}
                            Err(RecvError::Lagged(_)) => refresh = true,
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if refresh {
                    let outputs = normalize_outputs_response(ctx.state.output_controller.list_outputs(&ctx.state));
                    let json = serde_json::to_string(&outputs).unwrap_or_else(|_| "null".to_string());
                    if ctx.last_outputs.as_deref() != Some(json.as_str()) {
                        ctx.last_outputs = Some(json.clone());
                        ctx.pending.push_back(sse_event("outputs", &json));
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/metadata/stream",
    responses(
        (status = 200, description = "Metadata event stream")
    )
)]
#[get("/metadata/stream")]
/// Stream metadata job updates via server-sent events.
pub async fn metadata_stream(state: web::Data<AppState>) -> impl Responder {
    let receiver = state.events.subscribe();
    let mut pending = VecDeque::new();

    let stream = unfold(
        MetadataStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                tokio::select! {
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(HubEvent::Metadata(event)) => {
                                let json = serde_json::to_string(&event)
                                    .unwrap_or_else(|_| "null".to_string());
                                ctx.pending.push_back(sse_event("metadata", &json));
                            }
                            Ok(_) => {}
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/albums/stream",
    responses(
        (status = 200, description = "Album change event stream")
    )
)]
#[get("/albums/stream")]
/// Stream album change notifications via server-sent events.
pub async fn albums_stream(state: web::Data<AppState>) -> impl Responder {
    let receiver = state.events.subscribe();
    let mut pending = VecDeque::new();

    let stream = unfold(
        AlbumsStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                tokio::select! {
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(HubEvent::LibraryChanged) => {
                                ctx.pending.push_back(sse_event("albums", "{}"));
                            }
                            Ok(_) => {}
                            Err(RecvError::Lagged(_)) => {
                                ctx.pending.push_back(sse_event("albums", "{}"));
                            }
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/logs/stream",
    responses(
        (status = 200, description = "Server log event stream")
    )
)]
#[get("/logs/stream")]
/// Stream server logs via server-sent events.
pub async fn logs_stream(state: web::Data<AppState>) -> impl Responder {
    let initial = state.log_bus.snapshot();
    let initial_json = serde_json::to_string(&initial).unwrap_or_else(|_| "[]".to_string());
    let mut pending = VecDeque::new();
    pending.push_back(sse_event("logs", &initial_json));

    let receiver = state.log_bus.subscribe();
    let stream = unfold(
        LogsStreamState {
            receiver,
            pending,
            last_ping: Instant::now(),
        },
        |mut ctx| async move {
            loop {
                if let Some(bytes) = ctx.pending.pop_front() {
                    return Some((Ok::<Bytes, Error>(bytes), ctx));
                }
                tokio::select! {
                    result = ctx.receiver.recv() => {
                        match result {
                            Ok(event) => {
                                let json = serde_json::to_string(&event)
                                    .unwrap_or_else(|_| "null".to_string());
                                ctx.pending.push_back(sse_event("log", &json));
                            }
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => return None,
                        }
                    }
                }

                if ctx.pending.is_empty() && ctx.last_ping.elapsed() >= Duration::from_secs(15) {
                    ctx.last_ping = Instant::now();
                    ctx.pending.push_back(Bytes::from(": ping\n\n"));
                }
            }
        },
    );

    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "text/event-stream"))
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::CONNECTION, "keep-alive"))
        .streaming(stream)
}

#[utoipa::path(
    get,
    path = "/providers",
    responses(
        (status = 200, description = "Available output providers", body = ProvidersResponse)
    )
)]
#[get("/providers")]
/// List all available output providers.
pub async fn providers_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(state.output_controller.list_providers(&state))
}

#[utoipa::path(
    get,
    path = "/providers/{id}/outputs",
    responses(
        (status = 200, description = "Provider outputs", body = OutputsResponse),
        (status = 400, description = "Unknown provider"),
        (status = 500, description = "Provider unavailable")
    )
)]
#[get("/providers/{id}/outputs")]
/// List outputs for the requested provider.
pub async fn provider_outputs_list(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    match state
        .output_controller
        .outputs_for_provider(&state, id.as_str())
        .await
    {
        Ok(resp) => HttpResponse::Ok().json(resp),
        Err(err) => err.into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/outputs",
    responses(
        (status = 200, description = "Available outputs", body = OutputsResponse)
    )
)]
#[get("/outputs")]
/// List all outputs across providers.
pub async fn outputs_list(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(normalize_outputs_response(
        state.output_controller.list_outputs(&state),
    ))
}

#[utoipa::path(
    post,
    path = "/outputs/select",
    request_body = OutputSelectRequest,
    responses(
        (status = 200, description = "Active output set"),
        (status = 400, description = "Unknown output")
    )
)]
#[post("/outputs/select")]
/// Select the active output.
pub async fn outputs_select(
    state: web::Data<AppState>,
    body: web::Json<OutputSelectRequest>,
) -> impl Responder {
    match state.output_controller.select_output(&state, &body.id).await {
        Ok(()) => {
            state.events.outputs_changed();
            HttpResponse::Ok().finish()
        }
        Err(err) => err.into_response(),
    }
}

fn parse_single_range(header: &str, total_len: u64) -> Option<(u64, u64)> {
    let header = header.trim();
    if !header.starts_with("bytes=") {
        return None;
    }
    let range = header.trim_start_matches("bytes=");
    let first = range.split(',').next()?;
    let (start_s, end_s) = first.split_once('-')?;
    if start_s.is_empty() {
        return None;
    }
    let start = start_s.parse::<u64>().ok()?;
    let end = if end_s.is_empty() {
        total_len.saturating_sub(1)
    } else {
        end_s.parse::<u64>().ok()?
    };
    if start >= total_len || end < start {
        return None;
    }
    Some((start, end.min(total_len.saturating_sub(1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_range_accepts_open_end() {
        let range = parse_single_range("bytes=10-", 100).unwrap();
        assert_eq!(range, (10, 99));
    }

    #[test]
    fn parse_single_range_rejects_invalid() {
        assert!(parse_single_range("items=1-2", 100).is_none());
        assert!(parse_single_range("bytes=-10", 100).is_none());
        assert!(parse_single_range("bytes=200-300", 100).is_none());
        assert!(parse_single_range("bytes=50-40", 100).is_none());
    }

    #[test]
    fn parse_single_range_clamps_end_to_length() {
        let range = parse_single_range("bytes=90-200", 100).unwrap();
        assert_eq!(range, (90, 99));
    }

    #[test]
    fn parse_single_range_accepts_exact_end() {
        let range = parse_single_range("bytes=0-0", 100).unwrap();
        assert_eq!(range, (0, 0));
    }

    #[test]
    fn parse_single_range_uses_first_range() {
        let range = parse_single_range("bytes=0-1,2-3", 100).unwrap();
        assert_eq!(range, (0, 1));
    }
}
