//! Metadata-related API handlers.

use actix_files::NamedFile;
use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::musicbrainz::MusicBrainzMatch;
use crate::models::{
    AlbumListResponse,
    AlbumMetadataResponse,
    AlbumMetadataUpdateRequest,
    ArtistListResponse,
    MusicBrainzMatchApplyRequest,
    MusicBrainzMatchCandidate,
    MusicBrainzMatchKind,
    MusicBrainzMatchSearchRequest,
    MusicBrainzMatchSearchResponse,
    TrackListResponse,
    TrackMetadataResponse,
    TrackMetadataUpdateRequest,
    TrackResolveResponse,
};
use crate::state::AppState;
use crate::tag_writer::{write_track_tags, TrackTagUpdate};

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
    let metadata_service = state.metadata_service();
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
    let metadata_service = state.metadata_service();
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
    let metadata_service = state.metadata_service();
    let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &request.path) {
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
    let metadata_service = state.metadata_service();
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
        let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &path) {
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
    let metadata_service = state.metadata_service();
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
    let metadata_service = state.metadata_service();
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
    let metadata_service = state.metadata_service();
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
    match state.metadata.db
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
    match state.metadata.db.list_albums(
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
    let Some(_) = state.metadata.musicbrainz.as_ref() else {
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
            let record = match state.metadata.db.track_record_by_path(&path) {
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
            if let Err(err) = state.metadata.db
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
                artist_name: None,
                artist_sort_name: None,
                album_mbid: Some(album_mbid),
                album_title: None,
                release_year,
                release_candidates: Vec::new(),
            };
            let override_existing = override_existing.unwrap_or(true);
            if let Err(err) = state.metadata.db
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
