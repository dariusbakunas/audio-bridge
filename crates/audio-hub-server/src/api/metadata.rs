//! Metadata-related API handlers.

use actix_files::NamedFile;
use actix_web::{HttpRequest, HttpResponse, Responder, get, post, web};
use serde::Deserialize;
use std::time::Instant;
use utoipa::{IntoParams, ToSchema};

use crate::media_assets::MediaAssetStore;
use crate::metadata_db::{MediaAssetRecord, TextEntry};
use crate::models::{
    AlbumImageClearRequest, AlbumImageSetRequest, AlbumListResponse, AlbumMetadataResponse,
    AlbumMetadataUpdateRequest, AlbumMetadataUpdateResponse, AlbumProfileResponse,
    AlbumProfileUpdateRequest, ArtistImageClearRequest, ArtistImageSetRequest, ArtistListResponse,
    ArtistProfileResponse, ArtistProfileUpdateRequest, MediaAssetInfo,
    MusicBrainzMatchApplyRequest, MusicBrainzMatchCandidate, MusicBrainzMatchKind,
    MusicBrainzMatchSearchRequest, MusicBrainzMatchSearchResponse, TextMetadata,
    TrackAnalysisHeuristics, TrackAnalysisRequest, TrackAnalysisResponse, TrackListResponse,
    TrackMetadataFieldsResponse, TrackMetadataResponse, TrackMetadataUpdateRequest,
    TrackResolveResponse,
};
use crate::musicbrainz::MusicBrainzMatch;
use crate::state::AppState;
use crate::tag_writer::{
    TrackTagUpdate, read_editable_vorbis_tags, supported_track_fields, tag_type_label,
    write_track_tags,
};
use crate::track_analysis::{AnalysisOptions, analyze_track};
use base64::{Engine as _, engine::general_purpose};

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

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct TrackResolveQuery {
    pub track_id: i64,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct TrackMetadataQuery {
    pub track_id: i64,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct AlbumMetadataQuery {
    pub album_id: i64,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct ArtistProfileQuery {
    pub artist_id: i64,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct AlbumProfileQuery {
    pub album_id: i64,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct MediaAssetPath {
    pub id: i64,
}

const DEFAULT_LANG: &str = "en-US";

fn map_text_metadata(entry: TextEntry) -> TextMetadata {
    TextMetadata {
        text: entry.text,
        source: entry.source,
        locked: entry.locked,
        updated_at_ms: entry.updated_at_ms,
    }
}

fn map_media_asset_info(entry: MediaAssetRecord) -> MediaAssetInfo {
    MediaAssetInfo {
        id: entry.id,
        url: format!("/media/{}", entry.id),
        checksum: entry.checksum,
        source_url: entry.source_url,
        updated_at_ms: entry.updated_at_ms,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
    let metadata_service = state.metadata_service();
    let path = match state.metadata.db.track_path_for_id(query.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id = query.track_id,
                reason = "track_id_not_found",
                "tracks resolve failed"
            );
            return HttpResponse::NotFound().finish();
        }
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    match metadata_service.album_id_for_track_path(&path) {
        Ok(Some(album_id)) => HttpResponse::Ok().json(TrackResolveResponse {
            album_id: Some(album_id),
        }),
        Ok(None) => {
            tracing::warn!(track_id = query.track_id, track_path = %path, reason = "album_not_found_for_track", "tracks resolve failed");
            HttpResponse::NotFound().finish()
        }
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
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = state.metadata_service();
    let record = metadata_service.track_record_by_id(query.track_id);
    match record {
        Ok(Some(record)) => {
            let mut extra_tags = std::collections::BTreeMap::new();
            if let Ok(full_path) =
                crate::metadata_service::MetadataService::resolve_track_path(&root, &record.path)
            {
                match read_editable_vorbis_tags(&full_path) {
                    Ok(tags) => extra_tags = tags,
                    Err(err) => {
                        tracing::warn!(error = %err, path = %record.path, "read vorbis tags failed");
                    }
                }
            }
            HttpResponse::Ok().json(TrackMetadataResponse {
                track_id: query.track_id,
                title: record.title,
                artist: record.artist,
                album: record.album,
                album_artist: record.album_artist,
                year: record.year,
                track_number: record.track_number,
                disc_number: record.disc_number,
                extra_tags,
            })
        }
        Ok(None) => {
            tracing::warn!(
                track_id = query.track_id,
                reason = "track_id_not_found",
                "tracks metadata missing"
            );
            HttpResponse::NotFound().finish()
        }
        Err(err) => HttpResponse::InternalServerError().body(err),
    }
}

#[utoipa::path(
    get,
    path = "/tracks/metadata/fields",
    params(TrackMetadataQuery),
    responses(
        (status = 200, description = "Track metadata fields", body = TrackMetadataFieldsResponse),
        (status = 404, description = "Track not found")
    )
)]
#[get("/tracks/metadata/fields")]
/// Return supported tag fields for a track path.
pub async fn tracks_metadata_fields(
    state: web::Data<AppState>,
    query: web::Query<TrackMetadataQuery>,
) -> impl Responder {
    let root = state.library.read().unwrap().root().to_path_buf();
    let path = match state.metadata.db.track_path_for_id(query.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id = query.track_id,
                reason = "track_id_not_found",
                "tracks metadata fields missing"
            );
            return HttpResponse::NotFound().finish();
        }
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &path)
    {
        Ok(path) => path,
        Err(response) => return response,
    };

    let (tag_type, fields) = supported_track_fields(&full_path);
    let tag_label = tag_type.map(tag_type_label).map(|s| s.to_string());
    HttpResponse::Ok().json(TrackMetadataFieldsResponse {
        tag_type: tag_label,
        fields,
    })
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
    let metadata_service = state.metadata_service();
    let path = match state.metadata.db.track_path_for_id(request.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id = request.track_id,
                reason = "track_id_not_found",
                "tracks metadata update failed"
            );
            return HttpResponse::NotFound().finish();
        }
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &path)
    {
        Ok(path) => path,
        Err(response) => return response,
    };

    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let artist = request
        .artist
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let album = request
        .album
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let album_artist = request
        .album_artist
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let year = request.year.filter(|value| *value > 0);
    let track_number = request.track_number.filter(|value| *value > 0);
    let disc_number = request.disc_number.filter(|value| *value > 0);
    let clear_fields = request
        .clear_fields
        .unwrap_or_default()
        .into_iter()
        .map(|field| field.trim().to_ascii_lowercase())
        .filter(|field| !field.is_empty())
        .collect::<std::collections::HashSet<_>>();
    let clear_title = clear_fields.contains("title");
    let clear_artist = clear_fields.contains("artist");
    let clear_album = clear_fields.contains("album");
    let clear_album_artist = clear_fields.contains("album_artist");
    let clear_year = clear_fields.contains("year");
    let clear_track_number = clear_fields.contains("track_number");
    let clear_disc_number = clear_fields.contains("disc_number");
    let extra_tags = request
        .extra_tags
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_ascii_uppercase();
            let value = value.trim().to_string();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let clear_extra_tags = request
        .clear_extra_tags
        .unwrap_or_default()
        .into_iter()
        .map(|key| key.trim().to_ascii_uppercase())
        .filter(|key| !key.is_empty())
        .collect::<std::collections::HashSet<_>>();

    if title.is_none()
        && artist.is_none()
        && album.is_none()
        && album_artist.is_none()
        && year.is_none()
        && track_number.is_none()
        && disc_number.is_none()
        && extra_tags.is_empty()
        && !clear_title
        && !clear_artist
        && !clear_album
        && !clear_album_artist
        && !clear_year
        && !clear_track_number
        && !clear_disc_number
        && clear_extra_tags.is_empty()
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
            extra_tags: Some(&extra_tags),
            clear_title,
            clear_artist,
            clear_album,
            clear_album_artist,
            clear_year,
            clear_track_number,
            clear_disc_number,
            clear_extra_tags: Some(&clear_extra_tags),
        },
    ) {
        tracing::warn!(error = %err, path = %path, "track metadata update failed");
        return HttpResponse::InternalServerError().body(err.to_string());
    }

    if let Err(response) = metadata_service.rescan_track(&state.library, &full_path) {
        return response;
    }

    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/tracks/analysis",
    request_body = TrackAnalysisRequest,
    responses(
        (status = 200, description = "Track analysis", body = TrackAnalysisResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Track not found")
    )
)]
#[post("/tracks/analysis")]
/// Run on-demand analysis (spectrogram + heuristics) for a track.
pub async fn tracks_analysis(
    state: web::Data<AppState>,
    body: web::Json<TrackAnalysisRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let root = state.library.read().unwrap().root().to_path_buf();
    let path = match state.metadata.db.track_path_for_id(request.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id = request.track_id,
                "track analysis track id not found"
            );
            return HttpResponse::NotFound().body("track_id not found");
        }
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &path)
    {
        Ok(path) => path,
        Err(response) => {
            tracing::warn!(
                track_id = request.track_id,
                track_path = %path,
                status = %response.status(),
                "track analysis path resolution failed"
            );
            return response;
        }
    };

    let max_seconds = match request.max_seconds {
        Some(value) if value > 0.0 => value.clamp(5.0, 1800.0),
        _ => 0.0,
    };
    let width = request.width.unwrap_or(480).clamp(120, 1024);
    let height = request.height.unwrap_or(128).clamp(64, 512);
    let window_size = request.window_size.unwrap_or(4096).clamp(2048, 8192);
    let high_cutoff_hz = request.high_cutoff_hz.filter(|value| *value > 1000.0);
    let analysis = match web::block(move || {
        analyze_track(
            &full_path,
            AnalysisOptions {
                max_seconds,
                width,
                height,
                window_size,
                high_cutoff_hz,
            },
        )
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => return HttpResponse::BadRequest().body(err.to_string()),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };

    let data_base64 = general_purpose::STANDARD.encode(&analysis.data);
    HttpResponse::Ok().json(TrackAnalysisResponse {
        width: analysis.width,
        height: analysis.height,
        sample_rate: analysis.sample_rate,
        duration_ms: analysis.duration_ms,
        data_base64,
        heuristics: TrackAnalysisHeuristics {
            rolloff_hz: analysis.heuristics.rolloff_hz,
            ultrasonic_ratio: analysis.heuristics.ultrasonic_ratio,
            upper_audible_ratio: analysis.heuristics.upper_audible_ratio,
            dynamic_range_db: analysis.heuristics.dynamic_range_db,
            notes: analysis.heuristics.notes,
        },
    })
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
    let metadata_service = state.metadata_service();
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
        (status = 200, description = "Album metadata updated", body = AlbumMetadataUpdateResponse),
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
    #[derive(Debug)]
    enum AlbumMetadataUpdateError {
        BadRequest(String),
        NotFound(String),
        Internal(String),
    }

    impl AlbumMetadataUpdateError {
        fn into_response(self) -> HttpResponse {
            match self {
                AlbumMetadataUpdateError::BadRequest(message) => {
                    HttpResponse::BadRequest().body(message)
                }
                AlbumMetadataUpdateError::NotFound(message) => {
                    HttpResponse::NotFound().body(message)
                }
                AlbumMetadataUpdateError::Internal(message) => {
                    HttpResponse::InternalServerError().body(message)
                }
            }
        }
    }

    let request = body.into_inner();
    let root = state.library.read().unwrap().root().to_path_buf();
    let metadata_service = state.metadata_service();
    let album = request
        .album
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
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

    if album.is_none() && album_artist.is_none() && year.is_none() && track_artist.is_none() {
        return HttpResponse::BadRequest().body("no metadata fields provided");
    }

    let paths = match metadata_service.list_track_paths_by_album_id(request.album_id) {
        Ok(paths) => paths,
        Err(err) => return HttpResponse::InternalServerError().body(err),
    };
    if paths.is_empty() {
        return HttpResponse::NotFound().finish();
    }

    let album_id = request.album_id;
    let track_count = paths.len();
    let album_owned = album.map(str::to_string);
    let album_artist_owned = album_artist.map(str::to_string);
    let track_artist_owned = track_artist.map(str::to_string);
    let started_at = Instant::now();
    tracing::info!(album_id, track_count, "album metadata update started");

    let state_for_update = state.clone();
    let update_result =
        tokio::task::spawn_blocking(move || -> Result<i64, AlbumMetadataUpdateError> {
            for path in paths {
                let full_path =
                    crate::metadata_service::MetadataService::resolve_track_path(&root, &path)
                        .map_err(|response| match response.status().as_u16() {
                            400 => AlbumMetadataUpdateError::BadRequest(format!(
                                "track path outside library root: {path}"
                            )),
                            404 => AlbumMetadataUpdateError::NotFound(format!(
                                "track not found during album update: {path}"
                            )),
                            _ => AlbumMetadataUpdateError::Internal(format!(
                                "failed to resolve track path for album update: {path}"
                            )),
                        })?;

                if let Err(err) = write_track_tags(
                    &full_path,
                    TrackTagUpdate {
                        title: None,
                        artist: track_artist_owned.as_deref(),
                        album: album_owned.as_deref(),
                        album_artist: album_artist_owned.as_deref(),
                        year,
                        track_number: None,
                        disc_number: None,
                        extra_tags: None,
                        clear_title: false,
                        clear_artist: false,
                        clear_album: false,
                        clear_album_artist: false,
                        clear_year: false,
                        clear_track_number: false,
                        clear_disc_number: false,
                        clear_extra_tags: None,
                    },
                ) {
                    return Err(AlbumMetadataUpdateError::Internal(format!(
                        "album metadata update failed for {path}: {err}"
                    )));
                }

                if let Err(response) =
                    metadata_service.rescan_track(&state_for_update.library, &full_path)
                {
                    return Err(match response.status().as_u16() {
                        400 => AlbumMetadataUpdateError::BadRequest(format!(
                            "track rescan rejected after tag update: {path}"
                        )),
                        404 => AlbumMetadataUpdateError::NotFound(format!(
                            "track disappeared during rescan: {path}"
                        )),
                        _ => AlbumMetadataUpdateError::Internal(format!(
                            "track rescan failed after tag update: {path}"
                        )),
                    });
                }
            }

            let mut updated_album_id = album_id;
            if album_owned.is_some() || album_artist_owned.is_some() || year.is_some() {
                match metadata_service.update_album_metadata(
                    album_id,
                    album_owned.as_deref(),
                    album_artist_owned.as_deref(),
                    year,
                ) {
                    Ok(Some(new_id)) => {
                        updated_album_id = new_id;
                    }
                    Ok(None) => {
                        return Err(AlbumMetadataUpdateError::NotFound(format!(
                            "album not found: {album_id}"
                        )));
                    }
                    Err(err) => {
                        return Err(AlbumMetadataUpdateError::Internal(format!(
                            "album metadata db update failed: {err}"
                        )));
                    }
                }
            }

            Ok(updated_album_id)
        })
        .await;

    match update_result {
        Ok(Ok(updated_album_id)) => {
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            tracing::info!(
                album_id,
                track_count,
                elapsed_ms,
                "album metadata update finished"
            );
            HttpResponse::Ok().json(AlbumMetadataUpdateResponse {
                album_id: updated_album_id,
            })
        }
        Ok(Err(err)) => {
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            tracing::warn!(album_id, track_count, elapsed_ms, error = ?err, "album metadata update failed");
            err.into_response()
        }
        Err(join_err) => {
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            tracing::error!(
                album_id,
                track_count,
                elapsed_ms,
                error = %join_err,
                "album metadata update worker panicked"
            );
            HttpResponse::InternalServerError().body("album metadata update worker failed")
        }
    }
}

#[utoipa::path(
    get,
    path = "/artists/profile",
    params(ArtistProfileQuery),
    responses(
        (status = 200, description = "Artist profile", body = ArtistProfileResponse),
        (status = 404, description = "Artist not found")
    )
)]
#[get("/artists/profile")]
/// Read artist profile metadata.
pub async fn artist_profile(
    state: web::Data<AppState>,
    query: web::Query<ArtistProfileQuery>,
) -> impl Responder {
    let lang = query.lang.as_deref().unwrap_or(DEFAULT_LANG).trim();
    if lang.is_empty() {
        return HttpResponse::BadRequest().body("lang is required");
    }
    let db = &state.metadata.db;
    match db.artist_exists(query.artist_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let bio = match db.artist_bio(query.artist_id, lang) {
        Ok(value) => value.map(map_text_metadata),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let image = match db.media_asset_for("artist", query.artist_id, "image") {
        Ok(value) => value.map(map_media_asset_info),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    HttpResponse::Ok().json(ArtistProfileResponse {
        artist_id: query.artist_id,
        lang: lang.to_string(),
        bio,
        image,
    })
}

#[utoipa::path(
    post,
    path = "/artists/profile/update",
    request_body = ArtistProfileUpdateRequest,
    responses(
        (status = 200, description = "Artist profile updated", body = ArtistProfileResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Artist not found")
    )
)]
#[post("/artists/profile/update")]
/// Update artist profile metadata.
pub async fn artist_profile_update(
    state: web::Data<AppState>,
    body: web::Json<ArtistProfileUpdateRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let lang = request.lang.as_deref().unwrap_or(DEFAULT_LANG).trim();
    if lang.is_empty() {
        return HttpResponse::BadRequest().body("lang is required");
    }
    let db = &state.metadata.db;
    match db.artist_exists(request.artist_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let mut updated = false;
    let source = request
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("manual");
    let updated_at_ms = now_ms();

    if let Some(bio) = request.bio {
        updated = true;
        let text = bio.trim();
        if text.is_empty() {
            if let Err(err) = db.delete_artist_bio(request.artist_id, lang) {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
        } else if let Err(err) = db.upsert_artist_bio(
            request.artist_id,
            lang,
            text,
            Some(source),
            request.bio_locked.unwrap_or(true),
            Some(updated_at_ms),
        ) {
            return HttpResponse::InternalServerError().body(err.to_string());
        }
    }

    if !updated {
        return HttpResponse::BadRequest().body("no profile fields provided");
    }

    let bio = match db.artist_bio(request.artist_id, lang) {
        Ok(value) => value.map(map_text_metadata),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let image = match db.media_asset_for("artist", request.artist_id, "image") {
        Ok(value) => value.map(map_media_asset_info),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    HttpResponse::Ok().json(ArtistProfileResponse {
        artist_id: request.artist_id,
        lang: lang.to_string(),
        bio,
        image,
    })
}

#[utoipa::path(
    get,
    path = "/albums/profile",
    params(AlbumProfileQuery),
    responses(
        (status = 200, description = "Album profile", body = AlbumProfileResponse),
        (status = 404, description = "Album not found")
    )
)]
#[get("/albums/profile")]
/// Read album profile metadata.
pub async fn album_profile(
    state: web::Data<AppState>,
    query: web::Query<AlbumProfileQuery>,
) -> impl Responder {
    let lang = query.lang.as_deref().unwrap_or(DEFAULT_LANG).trim();
    if lang.is_empty() {
        return HttpResponse::BadRequest().body("lang is required");
    }
    let db = &state.metadata.db;
    match db.album_exists(query.album_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let (original_year, edition_year, edition_label) = match db.album_edition_fields(query.album_id)
    {
        Ok(values) => values,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let notes = match db.album_notes(query.album_id, lang) {
        Ok(value) => value.map(map_text_metadata),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let image = match db.media_asset_for("album", query.album_id, "image") {
        Ok(value) => value.map(map_media_asset_info),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    HttpResponse::Ok().json(AlbumProfileResponse {
        album_id: query.album_id,
        lang: lang.to_string(),
        notes,
        original_year,
        edition_year,
        edition_label,
        image,
    })
}

#[utoipa::path(
    post,
    path = "/albums/profile/update",
    request_body = AlbumProfileUpdateRequest,
    responses(
        (status = 200, description = "Album profile updated", body = AlbumProfileResponse),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Album not found")
    )
)]
#[post("/albums/profile/update")]
/// Update album profile metadata.
pub async fn album_profile_update(
    state: web::Data<AppState>,
    body: web::Json<AlbumProfileUpdateRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let lang = request.lang.as_deref().unwrap_or(DEFAULT_LANG).trim();
    if lang.is_empty() {
        return HttpResponse::BadRequest().body("lang is required");
    }
    let db = &state.metadata.db;
    match db.album_exists(request.album_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let mut updated = false;
    let source = request
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("manual");
    let updated_at_ms = now_ms();

    if let Some(notes) = request.notes {
        updated = true;
        let text = notes.trim();
        if text.is_empty() {
            if let Err(err) = db.delete_album_notes(request.album_id, lang) {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
        } else if let Err(err) = db.upsert_album_notes(
            request.album_id,
            lang,
            text,
            Some(source),
            request.notes_locked.unwrap_or(true),
            Some(updated_at_ms),
        ) {
            return HttpResponse::InternalServerError().body(err.to_string());
        }
    }

    if request.original_year.is_some()
        || request.edition_year.is_some()
        || request.edition_label.is_some()
    {
        let (current_original, current_edition_year, current_edition_label) =
            match db.album_edition_fields(request.album_id) {
                Ok(values) => values,
                Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
            };
        let edition_label = request
            .edition_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                if request.edition_label.is_some() {
                    None
                } else {
                    current_edition_label
                }
            });
        let original_year = match request.original_year {
            Some(value) if value > 0 => Some(value),
            Some(_) => None,
            None => current_original,
        };
        let edition_year = match request.edition_year {
            Some(value) if value > 0 => Some(value),
            Some(_) => None,
            None => current_edition_year,
        };
        if let Err(err) = db.update_album_edition_fields(
            request.album_id,
            original_year,
            edition_year,
            edition_label.as_deref(),
        ) {
            return HttpResponse::InternalServerError().body(err.to_string());
        }
        updated = true;
    }

    if !updated {
        return HttpResponse::BadRequest().body("no profile fields provided");
    }

    let (original_year, edition_year, edition_label) =
        match db.album_edition_fields(request.album_id) {
            Ok(values) => values,
            Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
        };
    let notes = match db.album_notes(request.album_id, lang) {
        Ok(value) => value.map(map_text_metadata),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let image = match db.media_asset_for("album", request.album_id, "image") {
        Ok(value) => value.map(map_media_asset_info),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    HttpResponse::Ok().json(AlbumProfileResponse {
        album_id: request.album_id,
        lang: lang.to_string(),
        notes,
        original_year,
        edition_year,
        edition_label,
        image,
    })
}

#[utoipa::path(
    post,
    path = "/artists/image/set",
    request_body = ArtistImageSetRequest,
    responses(
        (status = 200, description = "Artist image updated", body = MediaAssetInfo),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Artist not found")
    )
)]
#[post("/artists/image/set")]
/// Fetch and store an artist image from a URL.
pub async fn artist_image_set(
    state: web::Data<AppState>,
    body: web::Json<ArtistImageSetRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let db = &state.metadata.db;
    match db.artist_exists(request.artist_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let root = state.library.read().unwrap().root().to_path_buf();
    let store = MediaAssetStore::new(root);
    let previous = match db.media_asset_for("artist", request.artist_id, "image") {
        Ok(value) => value,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let stored = match store
        .store_image_from_url("artist", request.artist_id, "image", &request.url)
        .await
    {
        Ok(value) => value,
        Err(err) => return HttpResponse::BadRequest().body(err.to_string()),
    };
    let id = match db.upsert_media_asset(
        "artist",
        request.artist_id,
        "image",
        &stored.local_path,
        Some(&stored.checksum),
        Some(&stored.source_url),
        Some(stored.updated_at_ms),
    ) {
        Ok(id) => id,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    if let Some(previous) = previous {
        if previous.local_path != stored.local_path {
            let _ = store.delete_asset_file(&previous.local_path);
        }
    }
    HttpResponse::Ok().json(MediaAssetInfo {
        id,
        url: format!("/media/{}", id),
        checksum: Some(stored.checksum),
        source_url: Some(stored.source_url),
        updated_at_ms: Some(stored.updated_at_ms),
    })
}

#[utoipa::path(
    post,
    path = "/artists/image/clear",
    request_body = ArtistImageClearRequest,
    responses(
        (status = 200, description = "Artist image cleared"),
        (status = 404, description = "Artist not found")
    )
)]
#[post("/artists/image/clear")]
/// Clear an artist image.
pub async fn artist_image_clear(
    state: web::Data<AppState>,
    body: web::Json<ArtistImageClearRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let db = &state.metadata.db;
    match db.artist_exists(request.artist_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let root = state.library.read().unwrap().root().to_path_buf();
    let store = MediaAssetStore::new(root);
    let previous = match db.delete_media_asset("artist", request.artist_id, "image") {
        Ok(value) => value,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    if let Some(previous) = previous {
        let _ = store.delete_asset_file(&previous.local_path);
    }
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/albums/image/set",
    request_body = AlbumImageSetRequest,
    responses(
        (status = 200, description = "Album image updated", body = MediaAssetInfo),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Album not found")
    )
)]
#[post("/albums/image/set")]
/// Fetch and store an album image from a URL.
pub async fn album_image_set(
    state: web::Data<AppState>,
    body: web::Json<AlbumImageSetRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let db = &state.metadata.db;
    match db.album_exists(request.album_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let root = state.library.read().unwrap().root().to_path_buf();
    let store = MediaAssetStore::new(root);
    let previous = match db.media_asset_for("album", request.album_id, "image") {
        Ok(value) => value,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let stored = match store
        .store_image_from_url("album", request.album_id, "image", &request.url)
        .await
    {
        Ok(value) => value,
        Err(err) => return HttpResponse::BadRequest().body(err.to_string()),
    };
    let id = match db.upsert_media_asset(
        "album",
        request.album_id,
        "image",
        &stored.local_path,
        Some(&stored.checksum),
        Some(&stored.source_url),
        Some(stored.updated_at_ms),
    ) {
        Ok(id) => id,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    if let Some(previous) = previous {
        if previous.local_path != stored.local_path {
            let _ = store.delete_asset_file(&previous.local_path);
        }
    }
    HttpResponse::Ok().json(MediaAssetInfo {
        id,
        url: format!("/media/{}", id),
        checksum: Some(stored.checksum),
        source_url: Some(stored.source_url),
        updated_at_ms: Some(stored.updated_at_ms),
    })
}

#[utoipa::path(
    post,
    path = "/albums/image/clear",
    request_body = AlbumImageClearRequest,
    responses(
        (status = 200, description = "Album image cleared"),
        (status = 404, description = "Album not found")
    )
)]
#[post("/albums/image/clear")]
/// Clear an album image.
pub async fn album_image_clear(
    state: web::Data<AppState>,
    body: web::Json<AlbumImageClearRequest>,
) -> impl Responder {
    let request = body.into_inner();
    let db = &state.metadata.db;
    match db.album_exists(request.album_id) {
        Ok(true) => {}
        Ok(false) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    }
    let root = state.library.read().unwrap().root().to_path_buf();
    let store = MediaAssetStore::new(root);
    let previous = match db.delete_media_asset("album", request.album_id, "image") {
        Ok(value) => value,
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    if let Some(previous) = previous {
        let _ = store.delete_asset_file(&previous.local_path);
    }
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    get,
    path = "/media/{id}",
    params(MediaAssetPath),
    responses(
        (status = 200, description = "Media asset"),
        (status = 404, description = "Asset not found")
    )
)]
#[get("/media/{id}")]
/// Serve a media asset by id.
pub async fn media_asset(
    state: web::Data<AppState>,
    path: web::Path<MediaAssetPath>,
    req: HttpRequest,
) -> impl Responder {
    let db = &state.metadata.db;
    let record = match db.media_asset_by_id(path.id) {
        Ok(Some(value)) => value,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let root = state.library.read().unwrap().root().to_path_buf();
    let store = MediaAssetStore::new(root);
    let full_path = match store.resolve_asset_path(&record.local_path) {
        Ok(path) => path,
        Err(_) => return HttpResponse::NotFound().finish(),
    };
    match NamedFile::open(full_path) {
        Ok(file) => file.into_response(&req),
        Err(_) => HttpResponse::NotFound().finish(),
    }
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
    let metadata_service = state.metadata_service();
    let cover_rel = match metadata_service.cover_path_for_track_id(path.id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id = path.id,
                reason = "cover_not_set",
                "track cover missing"
            );
            return HttpResponse::NotFound().finish();
        }
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
    let metadata_service = state.metadata_service();
    let cover_rel = match metadata_service.cover_path_for_album_id(path.id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                album_id = path.id,
                reason = "cover_not_set",
                "album cover missing"
            );
            return HttpResponse::NotFound().finish();
        }
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
        Err(err) => {
            tracing::warn!(cover_rel, error = %err, reason = "cover_path_not_found", "cover art canonicalize failed");
            return HttpResponse::NotFound().finish();
        }
    };
    if !full_path.starts_with(&art_root) {
        tracing::warn!(cover_rel, resolved = %full_path.display(), reason = "cover_outside_art_root", "cover art request forbidden");
        return HttpResponse::Forbidden().finish();
    }
    match NamedFile::open(full_path) {
        Ok(file) => file.into_response(req),
        Err(err) => {
            tracing::warn!(cover_rel, error = %err, reason = "cover_file_open_failed", "cover art open failed");
            HttpResponse::NotFound().finish()
        }
    }
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
pub async fn artists_list(
    state: web::Data<AppState>,
    query: web::Query<ListQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);
    match state
        .metadata
        .db
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
    match state
        .metadata
        .db
        .list_albums(query.artist_id, query.search.as_deref(), limit, offset)
    {
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
    match state.metadata.db.list_tracks(
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
    let Some(client) = state.metadata.musicbrainz.as_ref() else {
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
                Err(err) => {
                    tracing::warn!(error = %err, title, artist, "musicbrainz track search failed");
                    return HttpResponse::InternalServerError().body(err.to_string());
                }
            }
        }
        MusicBrainzMatchKind::Album => match client.search_releases(title, artist, limit) {
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
            Err(err) => {
                tracing::warn!(error = %err, title, artist, "musicbrainz album search failed");
                return HttpResponse::InternalServerError().body(err.to_string());
            }
        },
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
    let Some(_) = state.metadata.musicbrainz.as_ref() else {
        return HttpResponse::BadRequest().body("musicbrainz is disabled");
    };
    match body.into_inner() {
        MusicBrainzMatchApplyRequest::Track {
            track_id,
            recording_mbid,
            artist_mbid,
            album_mbid,
            release_year,
            override_existing,
        } => {
            tracing::info!(
                track_id = track_id,
                recording_mbid = %recording_mbid,
                artist_mbid = ?artist_mbid,
                album_mbid = ?album_mbid,
                release_year = ?release_year,
                override_existing = ?override_existing,
                "manual musicbrainz match apply (track)"
            );
            let record = match state.metadata.db.track_record_by_id(track_id) {
                Ok(Some(record)) => record,
                Ok(None) => return HttpResponse::NotFound().finish(),
                Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
            };
            let mb = MusicBrainzMatch {
                recording_mbid: Some(recording_mbid),
                artist_mbid,
                artist_sort_name: None,
                album_mbid,
                release_year,
                release_candidates: Vec::new(),
            };
            let override_existing = override_existing.unwrap_or(true);
            if let Err(err) =
                state
                    .metadata
                    .db
                    .apply_musicbrainz_with_override(&record, &mb, override_existing)
            {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
            tracing::info!(
                path = %record.path,
                album = ?record.album,
                "manual musicbrainz match applied (track)"
            );
            state
                .events
                .metadata_event(crate::events::MetadataEvent::MusicBrainzLookupSuccess {
                    track_id: Some(track_id),
                    recording_mbid: mb.recording_mbid.clone(),
                    artist_mbid: mb.artist_mbid.clone(),
                    album_mbid: mb.album_mbid.clone(),
                });
            state.metadata.wake.notify();
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
                artist_sort_name: None,
                album_mbid: Some(album_mbid),
                release_year,
                release_candidates: Vec::new(),
            };
            let override_existing = override_existing.unwrap_or(true);
            if let Err(err) =
                state
                    .metadata
                    .db
                    .apply_album_musicbrainz(album_id, &mb, override_existing)
            {
                return HttpResponse::InternalServerError().body(err.to_string());
            }
            tracing::info!(album_id, "manual musicbrainz match applied (album)");
            state.metadata.wake.notify();
        }
    }
    HttpResponse::Ok().finish()
}
