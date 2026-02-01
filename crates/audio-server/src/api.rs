use std::path::{Path, PathBuf};

use actix_web::{get, post, web, HttpResponse, Responder};
use serde::Deserialize;

use crate::library::scan_library;
use crate::models::{
    LibraryResponse,
    PlayRequest,
    QueueAddRequest,
    QueueItem,
    QueueRemoveRequest,
    QueueReplacePlayRequest,
    QueueResponse,
    StatusResponse,
};
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
    tracing::info!(root = %root.display(), "rescan requested");
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

    tracing::info!(path = %path.display(), "play request");
    {
        let mut queue = state.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == &path) {
            queue.index = Some(pos);
        } else {
            queue.index = None;
        }
    }
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
    tracing::info!("pause toggle request");
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
    tracing::info!("next request");
    if let Some(path) = advance_queue_for_api(&state) {
        return start_path(&state, path);
    }
    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::Next).is_ok() {
        return HttpResponse::Ok().finish();
    }
    HttpResponse::InternalServerError().body("player offline")
}

#[get("/queue")]
pub async fn queue_list(state: web::Data<AppState>) -> impl Responder {
    let queue = state.queue.lock().unwrap();
    let library = state.library.read().unwrap();
    let items = queue
        .items
        .iter()
        .map(|path| match library.find_track_by_path(path) {
            Some(crate::models::LibraryEntry::Track {
                path,
                file_name,
                duration_ms,
                sample_rate,
                album,
                artist,
                format,
                ..
            }) => QueueItem::Track {
                path,
                file_name,
                duration_ms,
                sample_rate,
                album,
                artist,
                format,
            },
            _ => QueueItem::Missing {
                path: path.to_string_lossy().to_string(),
            },
        })
        .collect();
    HttpResponse::Ok().json(QueueResponse {
        items,
        index: queue.index,
    })
}

#[post("/queue")]
pub async fn queue_add(state: web::Data<AppState>, body: web::Json<QueueAddRequest>) -> impl Responder {
    let mut added = 0usize;
    {
        let mut queue = state.queue.lock().unwrap();
        for path_str in &body.paths {
            let path = PathBuf::from(path_str);
            let path = match canonicalize_under_root(&state, &path) {
                Ok(dir) => dir,
                Err(_) => continue,
            };
            queue.items.push(path);
            added += 1;
        }
    }
    HttpResponse::Ok().body(format!("added {added}"))
}

#[post("/queue/remove")]
pub async fn queue_remove(state: web::Data<AppState>, body: web::Json<QueueRemoveRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut queue = state.queue.lock().unwrap();
    if let Some(pos) = queue.items.iter().position(|p| p == &path) {
        queue.items.remove(pos);
        if let Some(idx) = queue.index {
            if pos == idx {
                queue.index = None;
            } else if pos < idx {
                queue.index = Some(idx.saturating_sub(1));
            }
        }
    }
    HttpResponse::Ok().finish()
}

#[post("/queue/clear")]
pub async fn queue_clear(state: web::Data<AppState>) -> impl Responder {
    let mut queue = state.queue.lock().unwrap();
    queue.items.clear();
    queue.index = None;
    HttpResponse::Ok().finish()
}

#[post("/queue/next")]
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    if let Some(path) = advance_queue_for_api(&state) {
        return start_path(&state, path);
    }
    HttpResponse::NoContent().finish()
}

#[post("/queue/replace_play")]
pub async fn queue_replace_play(
    state: web::Data<AppState>,
    body: web::Json<QueueReplacePlayRequest>,
) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    {
        let mut queue = state.queue.lock().unwrap();
        queue.items.clear();
        queue.items.push(path.clone());
        queue.index = Some(0);
    }
    start_path(&state, path)
}

#[get("/status")]
pub async fn status(state: web::Data<AppState>) -> impl Responder {
    let status = state.status.lock().unwrap();
    let (title, artist, album, format, sample_rate) = match status.now_playing.as_ref() {
        Some(path) => {
            let lib = state.library.read().unwrap();
            match lib.find_track_by_path(path) {
                Some(crate::models::LibraryEntry::Track {
                    file_name,
                    sample_rate,
                    artist,
                    album,
                    format,
                    ..
                }) => (Some(file_name), artist, album, Some(format), sample_rate),
                _ => (None, None, None, None, None),
            }
        }
        None => (None, None, None, None, None),
    };
    let resp = StatusResponse {
        now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
        paused: status.paused,
        elapsed_ms: status.elapsed_ms,
        duration_ms: status.duration_ms,
        sample_rate,
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

fn advance_queue_for_api(state: &AppState) -> Option<PathBuf> {
    let mut queue = state.queue.lock().ok()?;
    if queue.items.is_empty() {
        return None;
    }
    let next_index = match queue.index {
        None => 0,
        Some(idx) => {
            if idx + 1 >= queue.items.len() {
                return None;
            }
            idx + 1
        }
    };
    queue.index = Some(next_index);
    queue.items.get(next_index).cloned()
}

fn start_path(state: &web::Data<AppState>, path: PathBuf) -> HttpResponse {
    let ext_hint = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if state
        .player
        .cmd_tx
        .send(crate::bridge::BridgeCommand::Play {
            path: path.clone(),
            ext_hint,
        })
        .is_ok()
    {
        if let Ok(mut s) = state.status.lock() {
            s.now_playing = Some(path);
            s.paused = false;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
}
