use actix_web::HttpResponse;

use crate::models::{OutputsResponse, ProvidersResponse, QueueItem, QueueMode, QueueResponse, StatusResponse};
use crate::output_providers::registry::OutputRegistry;
use crate::queue_playback::{dispatch_next_from_queue, NextDispatchResult};
use crate::state::AppState;

#[derive(Debug)]
pub(crate) enum OutputControllerError {
    NoActiveOutput,
    UnsupportedOutput { requested: String, active: String },
    OutputOffline { output_id: String },
    PlayerOffline,
    Http(HttpResponse),
}

impl OutputControllerError {
    pub(crate) fn into_response(self) -> HttpResponse {
        match self {
            OutputControllerError::NoActiveOutput => {
                HttpResponse::ServiceUnavailable().body("no active output selected")
            }
            OutputControllerError::UnsupportedOutput { requested, active } => {
                HttpResponse::BadRequest().body(format!(
                    "unsupported output id: {requested} (active: {active})"
                ))
            }
            OutputControllerError::OutputOffline { output_id } => {
                HttpResponse::ServiceUnavailable()
                    .body(format!("output offline: {output_id}"))
            }
            OutputControllerError::PlayerOffline => {
                HttpResponse::InternalServerError().body("player offline")
            }
            OutputControllerError::Http(resp) => resp,
        }
    }
}

pub(crate) struct OutputController {
    registry: OutputRegistry,
}

impl OutputController {
    pub(crate) fn new(registry: OutputRegistry) -> Self {
        Self { registry }
    }

    pub(crate) fn default() -> Self {
        Self::new(OutputRegistry::default())
    }

    pub(crate) async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), OutputControllerError> {
        self.registry
            .select_output(state, output_id)
            .await
            .map_err(OutputControllerError::Http)
    }

    pub(crate) async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, OutputControllerError> {
        self.registry
            .status_for_output(state, output_id)
            .await
            .map_err(OutputControllerError::Http)
    }

    pub(crate) fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, OutputControllerError> {
        self.registry
            .outputs_for_provider(state, provider_id)
            .map_err(OutputControllerError::Http)
    }

    pub(crate) fn list_outputs(&self, state: &AppState) -> OutputsResponse {
        self.registry.list_outputs(state)
    }

    pub(crate) fn list_providers(&self, state: &AppState) -> ProvidersResponse {
        self.registry.list_providers(state)
    }

    pub(crate) async fn ensure_active_output_connected(
        &self,
        state: &AppState,
    ) -> Result<(), OutputControllerError> {
        self.registry
            .ensure_active_connected(state)
            .await
            .map_err(OutputControllerError::Http)
    }

    pub(crate) async fn resolve_active_output_id(
        &self,
        state: &AppState,
        requested: Option<&str>,
    ) -> Result<String, OutputControllerError> {
        let active_id = state.bridge.bridges.lock().unwrap().active_output_id.clone();
        let Some(active_id) = active_id else {
            tracing::warn!("request rejected: no active output selected");
            return Err(OutputControllerError::NoActiveOutput);
        };
        if let Some(requested_id) = requested {
            if requested_id != active_id {
                tracing::warn!(
                    requested_id = %requested_id,
                    active_output_id = %active_id,
                    "request rejected: unsupported output id"
                );
                return Err(OutputControllerError::UnsupportedOutput {
                    requested: requested_id.to_string(),
                    active: active_id,
                });
            }
        }
        if let Err(_err) = self.ensure_active_output_connected(state).await {
            tracing::warn!(output_id = %active_id, "request rejected: output offline");
            return Err(OutputControllerError::OutputOffline { output_id: active_id });
        }
        Ok(active_id)
    }

    pub(crate) async fn play_request(
        &self,
        state: &AppState,
        path: std::path::PathBuf,
        queue_mode: QueueMode,
        requested_output: Option<&str>,
    ) -> Result<String, OutputControllerError> {
        match queue_mode {
            QueueMode::Keep => {
                let mut queue = state.bridge.queue.lock().unwrap();
                if let Some(pos) = queue.items.iter().position(|p| p == &path) {
                    queue.items.remove(pos);
                }
            }
            QueueMode::Replace => {
                let mut queue = state.bridge.queue.lock().unwrap();
                queue.items.clear();
            }
            QueueMode::Append => {
                let mut queue = state.bridge.queue.lock().unwrap();
                if !queue.items.iter().any(|p| p == &path) {
                    queue.items.push(path.clone());
                }
            }
        }

        let output_id = self
            .resolve_active_output_id(state, requested_output)
            .await?;
        self.dispatch_play(state, path.clone(), None, false)?;

        if let Ok(mut queue) = state.bridge.queue.lock() {
            if let Some(pos) = queue.items.iter().position(|p| p == &path) {
                queue.items.remove(pos);
            }
        }

        Ok(output_id)
    }

    pub(crate) fn queue_list(&self, state: &AppState) -> QueueResponse {
        let queue = state.bridge.queue.lock().unwrap();
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
        QueueResponse { items }
    }

    pub(crate) fn queue_add_paths(
        &self,
        state: &AppState,
        paths: Vec<String>,
    ) -> usize {
        let mut added = 0usize;
        let mut queue = state.bridge.queue.lock().unwrap();
        for path_str in paths {
            let path = std::path::PathBuf::from(path_str);
            let path = match self.canonicalize_under_root(state, &path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if queue.items.iter().any(|p| p == &path) {
                continue;
            }
            queue.items.push(path);
            added += 1;
        }
        added
    }

    pub(crate) fn queue_remove_path(
        &self,
        state: &AppState,
        path_str: &str,
    ) -> Result<bool, OutputControllerError> {
        let path = std::path::PathBuf::from(path_str);
        let path = self.canonicalize_under_root(state, &path)?;
        let mut queue = state.bridge.queue.lock().unwrap();
        if let Some(pos) = queue.items.iter().position(|p| p == &path) {
            queue.items.remove(pos);
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn queue_clear(&self, state: &AppState) {
        let mut queue = state.bridge.queue.lock().unwrap();
        queue.items.clear();
    }

    pub(crate) async fn queue_next(&self, state: &AppState) -> Result<bool, OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        match dispatch_next_from_queue(
            &state.bridge.queue,
            &state.bridge.status,
            &state.bridge.player.lock().unwrap().cmd_tx,
            false,
        ) {
            NextDispatchResult::Dispatched => Ok(true),
            NextDispatchResult::Empty => Ok(false),
            NextDispatchResult::Failed => Err(OutputControllerError::PlayerOffline),
        }
    }

    pub(crate) async fn pause_toggle(
        &self,
        state: &AppState,
    ) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        if state
            .bridge
            .player
            .lock()
            .unwrap()
            .cmd_tx
            .send(crate::bridge::BridgeCommand::PauseToggle)
            .is_ok()
        {
            if let Ok(mut s) = state.bridge.status.lock() {
                s.paused = !s.paused;
                s.user_paused = s.paused;
            }
            Ok(())
        } else {
            Err(OutputControllerError::PlayerOffline)
        }
    }

    pub(crate) async fn seek(
        &self,
        state: &AppState,
        ms: u64,
    ) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        if state
            .bridge
            .player
            .lock()
            .unwrap()
            .cmd_tx
            .send(crate::bridge::BridgeCommand::Seek { ms })
            .is_ok()
        {
            Ok(())
        } else {
            Err(OutputControllerError::PlayerOffline)
        }
    }

    fn dispatch_play(
        &self,
        state: &AppState,
        path: std::path::PathBuf,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<(), OutputControllerError> {
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let cmd = crate::bridge::BridgeCommand::Play {
            path: path.clone(),
            ext_hint,
            seek_ms,
            start_paused,
        };
        if state
            .bridge
            .player
            .lock()
            .unwrap()
            .cmd_tx
            .send(cmd)
            .is_ok()
        {
            if let Ok(mut s) = state.bridge.status.lock() {
                s.now_playing = Some(path);
                s.paused = start_paused;
                s.user_paused = start_paused;
            }
            Ok(())
        } else {
            Err(OutputControllerError::PlayerOffline)
        }
    }

    pub(crate) fn canonicalize_under_root(
        &self,
        state: &AppState,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf, OutputControllerError> {
        let root = state.library.read().unwrap().root().to_path_buf();
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let canon = candidate
            .canonicalize()
            .map_err(|_| OutputControllerError::Http(
                HttpResponse::BadRequest().body(format!("path does not exist: {:?}", path)),
            ))?;
        if !canon.starts_with(&root) {
            return Err(OutputControllerError::Http(
                HttpResponse::BadRequest()
                    .body(format!("path outside library root: {:?}", path)),
            ));
        }
        Ok(canon)
    }
}
