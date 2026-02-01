use std::path::{Path, PathBuf};

use actix_web::{get, post, web, HttpResponse, Responder};
use serde::Deserialize;

use crate::library::scan_library;
use crate::models::{LibraryResponse, PlayRequest, StatusResponse};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LibraryQuery {
    pub dir: Option<String>,
}

#[get("/library")]
pub async fn list_library(state: web::Data<AppState>, query: web::Query<LibraryQuery>) -> impl Responder {
    let dir = query
        .dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| state.library.read().unwrap().root().to_path_buf());

    let dir = match canonicalize_under_root(&state, &dir) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
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

#[post("/library/rescan")]
pub async fn rescan_library(state: web::Data<AppState>) -> impl Responder {
    let root = state.library.read().unwrap().root().to_path_buf();
    match scan_library(&root) {
        Ok(new_index) => {
            *state.library.write().unwrap() = new_index;
            HttpResponse::Ok().finish()
        }
        Err(e) => HttpResponse::InternalServerError().body(format!("scan failed: {e:#}")),
    }
}

#[post("/play")]
pub async fn play_track(state: web::Data<AppState>, body: web::Json<PlayRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };

    let ext_hint = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::Play { path: path.clone(), ext_hint }).is_ok() {
        if let Ok(mut s) = state.status.lock() {
            s.now_playing = Some(path);
            s.paused = false;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}

#[post("/pause")]
pub async fn pause_toggle(state: web::Data<AppState>) -> impl Responder {
    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::PauseToggle).is_ok() {
        if let Ok(mut s) = state.status.lock() {
            s.paused = !s.paused;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}

#[post("/next")]
pub async fn next_track(state: web::Data<AppState>) -> impl Responder {
    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::Next).is_ok() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}

#[get("/status")]
pub async fn status(state: web::Data<AppState>) -> impl Responder {
    let status = state.status.lock().unwrap();
    let (title, artist, album, format) = match status.now_playing.as_ref() {
        Some(path) => {
            let lib = state.library.read().unwrap();
            match lib.find_track_by_path(path) {
                Some(crate::models::LibraryEntry::Track {
                    file_name,
                    artist,
                    album,
                    format,
                    ..
                }) => (Some(file_name), artist, album, Some(format)),
                _ => (None, None, None, None),
            }
        }
        None => (None, None, None, None),
    };
    let resp = StatusResponse {
        now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
        paused: status.paused,
        elapsed_ms: status.elapsed_ms,
        duration_ms: status.duration_ms,
        title,
        artist,
        album,
        format,
    };
    HttpResponse::Ok().json(resp)
}

fn canonicalize_under_root(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    let root = state.library.read().unwrap().root().to_path_buf();
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canon = candidate
        .canonicalize()
        .map_err(|_| format!("path does not exist: {:?}", path))?;
    if !canon.starts_with(&root) {
        return Err(format!("path outside library root: {:?}", path));
    }
    Ok(canon)
}
