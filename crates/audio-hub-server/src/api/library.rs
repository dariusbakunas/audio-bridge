//! Library-related API handlers.

use std::path::PathBuf;

use actix_web::body::SizedStream;
use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, Responder, get, post, web};
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::process::Command;
use tokio_util::io::ReaderStream;
use utoipa::ToSchema;

use crate::models::LibraryResponse;
use crate::state::AppState;

/// Query parameters for library listing.
#[derive(Deserialize, ToSchema)]
pub struct LibraryQuery {
    /// Optional directory to list under the library root.
    pub dir: Option<String>,
}

/// Query parameters for transcode-by-id stream requests.
#[derive(Deserialize, ToSchema)]
pub struct TranscodeByIdQuery {
    /// Output format (mp3, opus, aac, wav).
    pub format: Option<String>,
    /// Optional audio bitrate in kbps (ignored for wav).
    pub bitrate_kbps: Option<u32>,
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
pub async fn list_library(
    state: web::Data<AppState>,
    query: web::Query<LibraryQuery>,
) -> impl Responder {
    let dir = query
        .dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.library.read().unwrap().root().to_path_buf());

    let dir = match state
        .output
        .controller
        .canonicalize_under_root(&state, &dir)
    {
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
    path = "/stream/track/{id}",
    params(
        ("id" = i64, Path, description = "Track id")
    ),
    responses(
        (status = 200, description = "Full file stream"),
        (status = 206, description = "Partial content"),
        (status = 404, description = "Not found"),
        (status = 416, description = "Invalid range")
    )
)]
#[get("/stream/track/{id}")]
/// Stream a track by id with HTTP range support.
pub async fn stream_track_id(
    state: web::Data<AppState>,
    req: HttpRequest,
    id: web::Path<i64>,
) -> impl Responder {
    let path = match path_for_track_id(&state, id.into_inner()) {
        Ok(path) => path,
        Err(resp) => return resp,
    };
    stream_file(&state, req, path).await
}

fn path_for_track_id(state: &web::Data<AppState>, track_id: i64) -> Result<PathBuf, HttpResponse> {
    let raw = match state.metadata.db.track_path_for_id(track_id) {
        Ok(Some(path)) => path,
        Ok(None) => {
            tracing::warn!(
                track_id,
                reason = "track_id_not_found",
                "stream track lookup failed"
            );
            return Err(HttpResponse::NotFound().finish());
        }
        Err(err) => return Err(HttpResponse::InternalServerError().body(err.to_string())),
    };
    let candidate = PathBuf::from(raw);
    state
        .output
        .controller
        .canonicalize_under_root(state, &candidate)
        .map_err(|err| err.into_response())
}

async fn stream_file(state: &web::Data<AppState>, req: HttpRequest, path: PathBuf) -> HttpResponse {
    let path = match state
        .output
        .controller
        .canonicalize_under_root(state, &path)
    {
        Ok(dir) => dir,
        Err(err) => return err.into_response(),
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, reason = "file_open_failed", "stream file open failed");
            return HttpResponse::NotFound().finish();
        }
    };
    let meta = match file.metadata().await {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, reason = "metadata_read_failed", "stream file metadata failed");
            return HttpResponse::NotFound().finish();
        }
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

    let content_type = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "opus" => "audio/ogg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    };

    let mut resp = HttpResponse::build(status_code);
    resp.insert_header((header::ACCEPT_RANGES, "bytes"));
    resp.insert_header((header::CONTENT_TYPE, content_type));
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
    get,
    path = "/stream/transcode/track/{id}",
    params(
        ("id" = i64, Path, description = "Track id"),
        ("format" = Option<String>, Query, description = "Output format: mp3, opus, aac, wav"),
        ("bitrate_kbps" = Option<u32>, Query, description = "Optional bitrate in kbps")
    ),
    responses(
        (status = 200, description = "Transcoded audio stream"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Track not found"),
        (status = 500, description = "Transcode failed")
    )
)]
#[get("/stream/transcode/track/{id}")]
/// Stream a transcoded audio track by track id (requires ffmpeg in PATH).
pub async fn transcode_track_id(
    state: web::Data<AppState>,
    id: web::Path<i64>,
    query: web::Query<TranscodeByIdQuery>,
) -> impl Responder {
    let path = match path_for_track_id(&state, id.into_inner()) {
        Ok(path) => path,
        Err(resp) => return resp,
    };
    let format = query.format.as_deref().unwrap_or("mp3");
    let bitrate_kbps = query.bitrate_kbps;
    transcode_file(path, format, bitrate_kbps).await
}

async fn transcode_file(path: PathBuf, format: &str, bitrate_kbps: Option<u32>) -> HttpResponse {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostdin")
        .arg("-i")
        .arg(&path)
        .arg("-vn")
        .arg("-sn")
        .arg("-dn");

    let content_type = match format.to_ascii_lowercase().as_str() {
        "mp3" => {
            let bitrate = bitrate_kbps.unwrap_or(192);
            cmd.arg("-c:a")
                .arg("libmp3lame")
                .arg("-b:a")
                .arg(format!("{bitrate}k"))
                .arg("-f")
                .arg("mp3");
            "audio/mpeg"
        }
        "opus" => {
            let bitrate = bitrate_kbps.unwrap_or(128);
            cmd.arg("-c:a")
                .arg("libopus")
                .arg("-b:a")
                .arg(format!("{bitrate}k"))
                .arg("-f")
                .arg("ogg");
            "audio/ogg"
        }
        "aac" => {
            let bitrate = bitrate_kbps.unwrap_or(192);
            cmd.arg("-c:a")
                .arg("aac")
                .arg("-b:a")
                .arg(format!("{bitrate}k"))
                .arg("-f")
                .arg("adts");
            "audio/aac"
        }
        "wav" => {
            cmd.arg("-c:a").arg("pcm_s16le").arg("-f").arg("wav");
            "audio/wav"
        }
        _ => {
            return HttpResponse::BadRequest().body("invalid format (use mp3, opus, aac, wav)");
        }
    };

    cmd.arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return HttpResponse::InternalServerError()
                .body(format!("failed to start ffmpeg: {err}"));
        }
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return HttpResponse::InternalServerError().body("failed to capture ffmpeg output");
        }
    };

    actix_web::rt::spawn(async move {
        let _ = child.wait().await;
    });

    let stream = ReaderStream::new(stdout);
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, content_type))
        .streaming(stream)
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
    let metadata_service = state.metadata_service();
    tracing::info!(root = %root.display(), "rescan requested");
    match metadata_service.rescan_library(true) {
        Ok(new_index) => {
            *state.library.write().unwrap() = new_index;
            state.events.library_changed();
            state.metadata.wake.notify();
            HttpResponse::Ok().finish()
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("scan failed: {e:#}")),
    }
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct RescanTrackRequest {
    pub track_id: i64,
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
    let metadata_service = state.metadata_service();
    let path = match state.metadata.db.track_path_for_id(body.track_id) {
        Ok(Some(path)) => path,
        Ok(None) => return HttpResponse::NotFound().body("track not found"),
        Err(err) => return HttpResponse::InternalServerError().body(err.to_string()),
    };
    let root = state.library.read().unwrap().root().to_path_buf();
    let full_path = match crate::metadata_service::MetadataService::resolve_track_path(&root, &path)
    {
        Ok(path) => path,
        Err(response) => return response,
    };
    if let Err(response) = metadata_service.rescan_track(&state.library, &full_path) {
        return response;
    }
    HttpResponse::Ok().finish()
}

pub(crate) fn parse_single_range(header: &str, total_len: u64) -> Option<(u64, u64)> {
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
    use super::parse_single_range;

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
