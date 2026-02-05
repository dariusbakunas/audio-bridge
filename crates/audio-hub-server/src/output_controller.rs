//! Output routing and control facade.
//!
//! Centralizes output selection, status, and playback commands across providers.

use actix_web::HttpResponse;

use crate::models::{OutputsResponse, ProvidersResponse, QueueMode, QueueResponse, StatusResponse};
use crate::output_providers::registry::OutputRegistry;
use crate::queue_service::NextDispatchResult;
use crate::state::{AppState, QueueState};

/// Errors returned by the output controller facade.
#[derive(Debug)]
pub(crate) enum OutputControllerError {
    NoActiveOutput,
    UnsupportedOutput { requested: String, active: String },
    OutputOffline { output_id: String },
    PlayerOffline,
    Http(HttpResponse),
}

impl OutputControllerError {
    /// Convert a controller error into an HTTP response.
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

/// Facade for output selection, status, and playback orchestration.
pub(crate) struct OutputController {
    registry: OutputRegistry,
}

fn apply_queue_mode(queue: &mut QueueState, path: &std::path::Path, mode: QueueMode) {
    match mode {
        QueueMode::Keep => {}
        QueueMode::Replace => {
            queue.items.clear();
        }
        QueueMode::Append => {
            if !queue.items.iter().any(|p| p == path) {
                queue.items.push(path.to_path_buf());
            }
        }
    }
}

impl OutputController {
    /// Build a controller around the provided registry.
    pub(crate) fn new(registry: OutputRegistry) -> Self {
        Self { registry }
    }

    /// Construct a controller with the default provider registry.
    pub(crate) fn default() -> Self {
        Self::new(OutputRegistry::default())
    }

    /// Switch the active output to the given id.
    pub(crate) async fn select_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<(), OutputControllerError> {
        self.registry
            .select_output(state, output_id)
            .await
            .map_err(|e| OutputControllerError::Http(e.into_response()))
    }

    /// Fetch status for a specific output id.
    pub(crate) async fn status_for_output(
        &self,
        state: &AppState,
        output_id: &str,
    ) -> Result<StatusResponse, OutputControllerError> {
        self.registry
            .status_for_output(state, output_id)
            .await
            .map_err(|e| OutputControllerError::Http(e.into_response()))
    }

    /// List outputs owned by a provider.
    pub(crate) async fn outputs_for_provider(
        &self,
        state: &AppState,
        provider_id: &str,
    ) -> Result<OutputsResponse, OutputControllerError> {
        self.registry
            .outputs_for_provider(state, provider_id)
            .await
            .map_err(|e| OutputControllerError::Http(e.into_response()))
    }

    /// Return the list of known outputs (all providers).
    pub(crate) fn list_outputs(&self, state: &AppState) -> OutputsResponse {
        self.registry.list_outputs(state)
    }

    /// Return the list of providers.
    pub(crate) fn list_providers(&self, state: &AppState) -> ProvidersResponse {
        self.registry.list_providers(state)
    }

    /// Ensure the active output is reachable before dispatching playback.
    pub(crate) async fn ensure_active_output_connected(
        &self,
        state: &AppState,
    ) -> Result<(), OutputControllerError> {
        self.registry
            .ensure_active_connected(state)
            .await
            .map_err(|e| OutputControllerError::Http(e.into_response()))
    }

    /// Validate and return the active output id, optionally checking a requested id.
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

    /// Handle a play request including queue mode updates and dispatch.
    pub(crate) async fn play_request(
        &self,
        state: &AppState,
        path: std::path::PathBuf,
        queue_mode: QueueMode,
        requested_output: Option<&str>,
    ) -> Result<String, OutputControllerError> {
        {
            let mut queue = state.playback_manager.queue_service().queue().lock().unwrap();
            apply_queue_mode(&mut queue, &path, queue_mode);
        }

        let output_id = self
            .resolve_active_output_id(state, requested_output)
            .await?;
        self.dispatch_play(state, path.clone(), None, false)?;

        Ok(output_id)
    }

    /// Return the current queue as API response items.
    pub(crate) fn queue_list(&self, state: &AppState) -> QueueResponse {
        state.playback_manager.queue_service().list(&state.library.read().unwrap())
    }

    /// Add paths to the queue and return the number added.
    pub(crate) fn queue_add_paths(
        &self,
        state: &AppState,
        paths: Vec<String>,
    ) -> usize {
        let mut resolved = Vec::new();
        for path_str in paths {
            let path = std::path::PathBuf::from(path_str);
            let path = match self.canonicalize_under_root(state, &path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            resolved.push(path);
        }
        state.playback_manager.queue_service().add_paths(resolved)
    }

    /// Remove a path from the queue.
    pub(crate) fn queue_remove_path(
        &self,
        state: &AppState,
        path_str: &str,
    ) -> Result<bool, OutputControllerError> {
        let path = std::path::PathBuf::from(path_str);
        let path = self.canonicalize_under_root(state, &path)?;
        Ok(state.playback_manager.queue_service().remove_path(&path))
    }

    /// Clear the queue.
    pub(crate) fn queue_clear(&self, state: &AppState) {
        state.playback_manager.queue_service().clear();
    }

    /// Dispatch the next queued track if available.
    pub(crate) async fn queue_next(&self, state: &AppState) -> Result<bool, OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        match state.playback_manager.queue_next() {
            NextDispatchResult::Dispatched => Ok(true),
            NextDispatchResult::Empty => Ok(false),
            NextDispatchResult::Failed => Err(OutputControllerError::PlayerOffline),
        }
    }

    /// Toggle pause/resume on the active output.
    pub(crate) async fn pause_toggle(
        &self,
        state: &AppState,
    ) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state
            .playback_manager
            .pause_toggle()
            .map_err(|_| OutputControllerError::PlayerOffline)?;
        Ok(())
    }

    /// Seek the active output to the requested position.
    pub(crate) async fn seek(
        &self,
        state: &AppState,
        ms: u64,
    ) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state
            .playback_manager
            .seek(ms)
            .map_err(|_| OutputControllerError::PlayerOffline)?;
        Ok(())
    }

    /// Stop playback on the active output.
    pub(crate) async fn stop(&self, state: &AppState) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state
            .playback_manager
            .stop()
            .map_err(|_| OutputControllerError::PlayerOffline)?;
        Ok(())
    }

    /// Dispatch a play request to the active transport.
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
        state
            .playback_manager
            .play(path, ext_hint, seek_ms, start_paused)
            .map_err(|_| OutputControllerError::PlayerOffline)
    }

    /// Canonicalize a path and ensure it is under the library root.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use async_trait::async_trait;
    use crate::models::OutputInfo;
    use crate::output_providers::registry::{OutputProvider, ProviderError};
    use crate::status_store::StatusStore;

    struct MockProvider {
        active_output_id: String,
        should_connect: bool,
    }

    #[async_trait]
    impl OutputProvider for MockProvider {
        fn list_providers(&self, _state: &AppState) -> Vec<crate::models::ProviderInfo> {
            Vec::new()
        }

        async fn outputs_for_provider(
            &self,
            _state: &AppState,
            _provider_id: &str,
        ) -> Result<OutputsResponse, ProviderError> {
            Ok(OutputsResponse {
                active_id: None,
                outputs: Vec::new(),
            })
        }

        fn list_outputs(&self, _state: &AppState) -> Vec<OutputInfo> {
            Vec::new()
        }

        fn can_handle_output_id(&self, output_id: &str) -> bool {
            output_id == self.active_output_id
        }

        fn can_handle_provider_id(&self, _state: &AppState, _provider_id: &str) -> bool {
            false
        }

        fn inject_active_output_if_missing(
            &self,
            _state: &AppState,
            _outputs: &mut Vec<OutputInfo>,
            _active_output_id: &str,
        ) {
        }

        async fn ensure_active_connected(&self, _state: &AppState) -> Result<(), ProviderError> {
            if self.should_connect {
                Ok(())
            } else {
                Err(ProviderError::Unavailable("offline".to_string()))
            }
        }

        async fn select_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<(), ProviderError> {
            Ok(())
        }

        async fn status_for_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<StatusResponse, ProviderError> {
            Err(ProviderError::Unavailable("offline".to_string()))
        }
    }

    fn make_state(active_output_id: Option<String>) -> AppState {
        let tmp = std::env::temp_dir()
            .join(format!(
                "audio-hub-server-test-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let _ = std::fs::create_dir_all(&tmp);
        let library = crate::library::scan_library(&tmp).expect("scan library");
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id,
        }));
        let bridge_state = Arc::new(crate::state::BridgeProviderState::new(
            cmd_tx,
            bridges_state,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            "http://localhost".to_string(),
        ));
        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(crate::state::LocalProviderState {
            enabled: false,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer {
                cmd_tx: local_cmd_tx,
            })),
            running: Arc::new(AtomicBool::new(false)),
        });
        let status = StatusStore::new(Arc::new(Mutex::new(crate::state::PlayerStatus::default())));
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(queue, status.clone());
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        AppState::new(
            library,
            bridge_state,
            local_state,
            playback_manager,
            device_selection,
        )
    }

    fn make_state_with_root() -> (AppState, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-server-root-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let library = crate::library::scan_library(&root).expect("scan library");
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id: None,
        }));
        let bridge_state = Arc::new(crate::state::BridgeProviderState::new(
            cmd_tx,
            bridges_state,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            "http://localhost".to_string(),
        ));
        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(crate::state::LocalProviderState {
            enabled: false,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer {
                cmd_tx: local_cmd_tx,
            })),
            running: Arc::new(AtomicBool::new(false)),
        });
        let status = StatusStore::new(Arc::new(Mutex::new(crate::state::PlayerStatus::default())));
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(queue, status.clone());
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let state = AppState::new(
            library,
            bridge_state,
            local_state,
            playback_manager,
            device_selection,
        );
        (state, root)
    }

    #[test]
    fn apply_queue_mode_keep_preserves_items() {
        let mut queue = QueueState {
            items: vec![std::path::PathBuf::from("/music/a.flac")],
        };
        apply_queue_mode(&mut queue, std::path::Path::new("/music/b.flac"), QueueMode::Keep);
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0], std::path::PathBuf::from("/music/a.flac"));
    }

    #[test]
    fn apply_queue_mode_replace_clears_queue() {
        let mut queue = QueueState {
            items: vec![std::path::PathBuf::from("/music/a.flac")],
        };
        apply_queue_mode(&mut queue, std::path::Path::new("/music/b.flac"), QueueMode::Replace);
        assert!(queue.items.is_empty());
    }

    #[test]
    fn apply_queue_mode_append_adds_once() {
        let mut queue = QueueState { items: Vec::new() };
        let path = std::path::Path::new("/music/a.flac");
        apply_queue_mode(&mut queue, path, QueueMode::Append);
        apply_queue_mode(&mut queue, path, QueueMode::Append);
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0], std::path::PathBuf::from("/music/a.flac"));
    }

    #[test]
    fn resolve_active_output_id_rejects_missing_active() {
        let state = make_state(None);
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));
        let result = actix_web::rt::System::new().block_on(async {
            controller.resolve_active_output_id(&state, None).await
        });
        assert!(matches!(result, Err(OutputControllerError::NoActiveOutput)));
    }

    #[test]
    fn resolve_active_output_id_rejects_mismatched_request() {
        let state = make_state(Some("bridge:test:device".to_string()));
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));
        let result = actix_web::rt::System::new().block_on(async {
            controller
                .resolve_active_output_id(&state, Some("bridge:other:device"))
                .await
        });
        assert!(matches!(
            result,
            Err(OutputControllerError::UnsupportedOutput { .. })
        ));
    }

    #[test]
    fn resolve_active_output_id_rejects_offline_output() {
        let active = "bridge:test:device".to_string();
        let state = make_state(Some(active.clone()));
        let provider = MockProvider {
            active_output_id: active,
            should_connect: false,
        };
        let controller = OutputController::new(OutputRegistry::new(vec![Box::new(provider)]));
        let result = actix_web::rt::System::new().block_on(async {
            controller.resolve_active_output_id(&state, None).await
        });
        assert!(matches!(
            result,
            Err(OutputControllerError::OutputOffline { .. })
        ));
    }

    #[test]
    fn canonicalize_under_root_accepts_relative_paths() {
        let (state, root) = make_state_with_root();
        let file_path = root.join("a.flac");
        let _ = std::fs::write(&file_path, b"test");
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let resolved = controller
            .canonicalize_under_root(&state, std::path::Path::new("a.flac"))
            .expect("canonicalize");

        assert_eq!(resolved, file_path.canonicalize().unwrap());
    }

    #[test]
    fn canonicalize_under_root_rejects_outside_root() {
        let (state, _root) = make_state_with_root();
        let other_root = std::env::temp_dir().join(format!(
            "audio-hub-server-outside-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&other_root);
        let outside = other_root.join("outside.flac");
        let _ = std::fs::write(&outside, b"test");
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let result = controller.canonicalize_under_root(&state, &outside);

        assert!(matches!(result, Err(OutputControllerError::Http(_))));
    }

    #[test]
    fn queue_add_paths_skips_missing_files() {
        let (state, root) = make_state_with_root();
        let file_path = root.join("a.flac");
        let _ = std::fs::write(&file_path, b"test");
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let added = controller.queue_add_paths(
            &state,
            vec!["a.flac".to_string(), "missing.flac".to_string()],
        );

        assert_eq!(added, 1);
        assert_eq!(
            state.playback_manager.queue_service().queue().lock().unwrap().items,
            vec![file_path.canonicalize().unwrap()]
        );
    }
}
