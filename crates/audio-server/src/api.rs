use std::path::{Path, PathBuf};

use actix_web::{get, post, web, HttpResponse, Responder};
use anyhow::Result;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::library::scan_library;
use crate::models::{
    LibraryResponse,
    PlayRequest,
    QueueMode,
    QueueAddRequest,
    QueueItem,
    QueueRemoveRequest,
    QueueResponse,
    StatusResponse,
    BridgeDevicesResponse,
    BridgeSetDeviceRequest,
    OutputsResponse,
    OutputSelectRequest,
};
use crate::state::AppState;

#[derive(Deserialize, ToSchema)]
pub struct LibraryQuery {
    pub dir: Option<String>,
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

#[utoipa::path(
    post,
    path = "/library/rescan",
    responses(
        (status = 200, description = "Rescan started"),
        (status = 500, description = "Rescan failed")
    )
)]
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
pub async fn play_track(state: web::Data<AppState>, body: web::Json<PlayRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };

    let mode = body.queue_mode.clone().unwrap_or(QueueMode::Keep);
    let output_id = match &body.output_id {
        Some(id) => id.clone(),
        None => state.outputs.lock().unwrap().active_id.clone(),
    };
    {
        let outputs = state.outputs.lock().unwrap();
        let Some(out) = outputs.outputs.iter().find(|o| o.id == output_id) else {
            return HttpResponse::BadRequest().body("unknown output id");
        };
        if out.kind != "bridge" || output_id != outputs.active_id {
            return HttpResponse::BadRequest().body("unsupported output id");
        }
    }
    match mode {
        QueueMode::Keep => {
            let mut queue = state.queue.lock().unwrap();
            if let Some(pos) = queue.items.iter().position(|p| p == &path) {
                queue.items.remove(pos);
            }
        }
        QueueMode::Replace => {
            let mut queue = state.queue.lock().unwrap();
            queue.items.clear();
        }
        QueueMode::Append => {
            let mut queue = state.queue.lock().unwrap();
            if !queue.items.iter().any(|p| p == &path) {
                queue.items.push(path.clone());
            }
        }
    }

    let ext_hint = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    tracing::info!(path = %path.display(), "play request");
    {
        let mut queue = state.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == &path) {
            queue.items.remove(pos);
        }
    }
    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::Play { path: path.clone(), ext_hint }).is_ok() {
        if let Ok(mut s) = state.status.lock() {
            s.now_playing = Some(path);
            s.paused = false;
            s.user_paused = false;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
    }
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
pub async fn pause_toggle(state: web::Data<AppState>) -> impl Responder {
    tracing::info!("pause toggle request");
    if state.player.cmd_tx.send(crate::bridge::BridgeCommand::PauseToggle).is_ok() {
        if let Ok(mut s) = state.status.lock() {
            s.paused = !s.paused;
            s.user_paused = s.paused;
        }
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::InternalServerError().body("player offline")
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
    HttpResponse::Ok().json(QueueResponse { items })
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
pub async fn queue_remove(state: web::Data<AppState>, body: web::Json<QueueRemoveRequest>) -> impl Responder {
    let path = PathBuf::from(&body.path);
    let path = match canonicalize_under_root(&state, &path) {
        Ok(dir) => dir,
        Err(e) => return HttpResponse::BadRequest().body(e),
    };
    let mut queue = state.queue.lock().unwrap();
    if let Some(pos) = queue.items.iter().position(|p| p == &path) {
        queue.items.remove(pos);
    }
    HttpResponse::Ok().finish()
}

#[utoipa::path(
    post,
    path = "/queue/clear",
    responses(
        (status = 200, description = "Queue cleared")
    )
)]
#[post("/queue/clear")]
pub async fn queue_clear(state: web::Data<AppState>) -> impl Responder {
    let mut queue = state.queue.lock().unwrap();
    queue.items.clear();
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
pub async fn queue_next(state: web::Data<AppState>) -> impl Responder {
    let path = {
        let mut queue = state.queue.lock().unwrap();
        if queue.items.is_empty() {
            None
        } else {
            Some(queue.items.remove(0))
        }
    };
    if let Some(path) = path {
        return start_path(&state, path);
    }
    HttpResponse::NoContent().finish()
}

#[utoipa::path(
    get,
    path = "/status",
    responses(
        (status = 200, description = "Playback status", body = StatusResponse)
    )
)]
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
    let output_id = state.outputs.lock().unwrap().active_id.clone();
    let resp = StatusResponse {
        now_playing: status.now_playing.as_ref().map(|p| p.to_string_lossy().to_string()),
        paused: status.paused,
        elapsed_ms: status.elapsed_ms,
        duration_ms: status.duration_ms,
        sample_rate,
        channels: status.channels,
        output_sample_rate: status.sample_rate,
        title,
        artist,
        album,
        format,
        output_id,
    };
    HttpResponse::Ok().json(resp)
}

#[utoipa::path(
    get,
    path = "/outputs/{id}/devices",
    responses(
        (status = 200, description = "Output devices", body = BridgeDevicesResponse),
        (status = 400, description = "Unknown output"),
        (status = 500, description = "Output unavailable")
    )
)]
#[get("/outputs/{id}/devices")]
pub async fn output_devices(
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    {
        let outputs = state.outputs.lock().unwrap();
        let Some(out) = outputs.outputs.iter().find(|o| o.id == id.as_str()) else {
            return HttpResponse::BadRequest().body("unknown output id");
        };
        if out.kind != "bridge" || out.id != outputs.active_id {
            return HttpResponse::BadRequest().body("unsupported output id");
        }
    }
    match bridge_list_devices(&state) {
        Ok(devices) => HttpResponse::Ok().json(BridgeDevicesResponse { devices }),
        Err(e) => HttpResponse::InternalServerError().body(format!("{e:#}")),
    }
}

#[utoipa::path(
    post,
    path = "/outputs/{id}/device",
    request_body = BridgeSetDeviceRequest,
    responses(
        (status = 200, description = "Device set"),
        (status = 400, description = "Unknown output"),
        (status = 500, description = "Output unavailable")
    )
)]
#[post("/outputs/{id}/device")]
pub async fn output_set_device(
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<BridgeSetDeviceRequest>,
) -> impl Responder {
    {
        let outputs = state.outputs.lock().unwrap();
        let Some(out) = outputs.outputs.iter().find(|o| o.id == id.as_str()) else {
            return HttpResponse::BadRequest().body("unknown output id");
        };
        if out.kind != "bridge" || out.id != outputs.active_id {
            return HttpResponse::BadRequest().body("unsupported output id");
        }
    }
    match bridge_set_device_req(&state, &body.name) {
        Ok(()) => HttpResponse::Ok().finish(),
        Err(e) => HttpResponse::InternalServerError().body(format!("{e:#}")),
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
pub async fn outputs_list(state: web::Data<AppState>) -> impl Responder {
    let state = state.outputs.lock().unwrap();
    let active_id = state.active_id.clone();
    let outputs = state.outputs.clone();
    HttpResponse::Ok().json(OutputsResponse { active_id, outputs })
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
pub async fn outputs_select(
    state: web::Data<AppState>,
    body: web::Json<OutputSelectRequest>,
) -> impl Responder {
    let mut state = state.outputs.lock().unwrap();
    let Some(out) = state.outputs.iter().find(|o| o.id == body.id) else {
        return HttpResponse::BadRequest().body("unknown output id");
    };
    if out.kind != "bridge" {
        return HttpResponse::BadRequest().body("unsupported output id");
    }
    state.active_id = body.id.clone();
    HttpResponse::Ok().finish()
}

fn bridge_list_devices(state: &AppState) -> Result<Vec<String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    state
        .player
        .cmd_tx
        .send(crate::bridge::BridgeCommand::ListDevices { resp_tx: tx })
        .map_err(|_| anyhow::anyhow!("bridge command channel closed"))?;
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| anyhow::anyhow!("bridge list timeout"))?
}

fn bridge_set_device_req(state: &AppState, name: &str) -> Result<()> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    state
        .player
        .cmd_tx
        .send(crate::bridge::BridgeCommand::SetDevice {
            name: name.to_string(),
            resp_tx: tx,
        })
        .map_err(|_| anyhow::anyhow!("bridge command channel closed"))?;
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| anyhow::anyhow!("bridge set timeout"))?
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
