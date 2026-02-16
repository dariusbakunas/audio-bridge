//! Output routing and control facade.
//!
//! Centralizes output selection, status, and playback commands across providers.

use actix_web::HttpResponse;

use crate::models::{OutputsResponse, ProvidersResponse, QueueMode, QueueResponse, StatusResponse};
use crate::output_providers::registry::OutputRegistry;
use crate::queue_service::NextDispatchResult;
use crate::state::{AppState};

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
    pub(crate) async fn list_outputs(&self, state: &AppState) -> OutputsResponse {
        self.registry.list_outputs(state).await
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
        let active_id = state.providers.bridge.bridges.lock().unwrap().active_output_id.clone();
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
        let _ = state.playback.manager.apply_queue_mode(&path, queue_mode);

        let output_id = self
            .resolve_active_output_id(state, requested_output)
            .await?;
        state.playback.manager.set_manual_advance_in_flight(true);
        self.dispatch_play(state, path.clone(), None, false)?;

        Ok(output_id)
    }

    /// Return the current queue as API response items.
    pub(crate) fn queue_list(&self, state: &AppState) -> QueueResponse {
        state
            .playback
            .manager
            .queue_list(&state.library.read().unwrap(), Some(&state.metadata.db))
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
        state.playback.manager.queue_add_paths(resolved)
    }

    /// Insert paths to the front of the queue and return the number added.
    pub(crate) fn queue_add_next_paths(
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
        state.playback.manager.queue_add_next_paths(resolved)
    }

    /// Remove a path from the queue.
    pub(crate) fn queue_remove_path(
        &self,
        state: &AppState,
        path_str: &str,
    ) -> Result<bool, OutputControllerError> {
        let path = std::path::PathBuf::from(path_str);
        let path = self.canonicalize_under_root(state, &path)?;
        Ok(state.playback.manager.queue_remove_path(&path))
    }

    /// Play a queued item and drop items ahead of it.
    pub(crate) async fn queue_play_from(
        &self,
        state: &AppState,
        path_str: &str,
    ) -> Result<bool, OutputControllerError> {
        let path = std::path::PathBuf::from(path_str);
        let path = self.canonicalize_under_root(state, &path)?;
        let found = state.playback.manager.queue_play_from(&path);
        if !found {
            return Ok(false);
        }
        let _ = self.resolve_active_output_id(state, None).await?;
        state.playback.manager.set_manual_advance_in_flight(true);
        self.dispatch_play(state, path.clone(), None, false)?;
        Ok(true)
    }

    /// Clear the queue.
    pub(crate) fn queue_clear(&self, state: &AppState) {
        state.playback.manager.queue_clear();
    }

    /// Dispatch the next queued track if available.
    pub(crate) async fn queue_next(&self, state: &AppState) -> Result<bool, OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state.playback.manager.set_manual_advance_in_flight(true);
        match state.playback.manager.queue_next() {
            NextDispatchResult::Dispatched => Ok(true),
            NextDispatchResult::Empty => {
                state.playback.manager.set_manual_advance_in_flight(false);
                Ok(false)
            }
            NextDispatchResult::Failed => {
                state.playback.manager.set_manual_advance_in_flight(false);
                Err(OutputControllerError::PlayerOffline)
            }
        }
    }

    /// Play the previous track from history if available.
    pub(crate) async fn queue_previous(&self, state: &AppState) -> Result<bool, OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        let current = state.playback.manager.current_path();
        let previous = state.playback.manager.take_previous(current.as_deref());
        let Some(path) = previous else {
            return Ok(false);
        };
        if let Some(current) = current {
            state.playback.manager.queue_add_next_paths(vec![current]);
        }
        state.playback.manager.set_manual_advance_in_flight(true);
        self.dispatch_play(state, path.clone(), None, false)?;
        state.playback.manager.update_has_previous();
        Ok(true)
    }

    /// Toggle pause/resume on the active output.
    pub(crate) async fn pause_toggle(
        &self,
        state: &AppState,
    ) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state.playback.manager
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
        state.playback.manager
            .seek(ms)
            .map_err(|_| OutputControllerError::PlayerOffline)?;
        Ok(())
    }

    /// Stop playback on the active output.
    pub(crate) async fn stop(&self, state: &AppState) -> Result<(), OutputControllerError> {
        let _ = self.resolve_active_output_id(state, None).await?;
        state.playback.manager
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
        state.playback.manager
            .play(path, ext_hint, seek_ms, start_paused)
            .map_err(|_| {
                let active_output_id = state
                    .providers
                    .bridge
                    .bridges
                    .lock()
                    .ok()
                    .and_then(|bridges| bridges.active_output_id.clone());
                let bridge_online = state.providers.bridge
                    .bridge_online
                    .load(std::sync::atomic::Ordering::Relaxed);
                let worker_running = state.providers.bridge
                    .worker_running
                    .load(std::sync::atomic::Ordering::Relaxed);
                let status_cached = state.providers.bridge
                    .status_cache
                    .lock()
                    .ok()
                    .map(|cache| !cache.is_empty())
                    .unwrap_or(false);
                tracing::warn!(
                    output_id = ?active_output_id,
                    bridge_online,
                    worker_running,
                    status_cached,
                    "play dispatch failed: player offline"
                );
                OutputControllerError::PlayerOffline
            })
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
    use crate::state::QueueState;
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

        async fn list_outputs(&self, _state: &AppState) -> Vec<OutputInfo> {
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

        async fn stop_output(
            &self,
            _state: &AppState,
            _output_id: &str,
        ) -> Result<(), ProviderError> {
            Ok(())
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
        let status = StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(
            queue,
            status.clone(),
            crate::events::EventBus::new(),
        );
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let metadata_db = crate::metadata_db::MetadataDb::new(library.root()).unwrap();
        let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
        AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
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
        let metadata_db = crate::metadata_db::MetadataDb::new(&root).unwrap();
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
        let status = StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(
            queue,
            status.clone(),
            crate::events::EventBus::new(),
        );
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
        let state = AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
        );
        (state, root)
    }

    #[test]
    fn apply_queue_mode_keep_preserves_items() {
        let (state, _root) = make_state_with_root();
        {
            let mut queue = state.playback.manager.queue_service().queue().lock().unwrap();
            queue.items.push(std::path::PathBuf::from("/music/a.flac"));
        }
        state
            .playback
            .manager
            .apply_queue_mode(std::path::Path::new("/music/b.flac"), QueueMode::Keep);
        let queue = state.playback.manager.queue_service().queue().lock().unwrap();
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0], std::path::PathBuf::from("/music/a.flac"));
    }

    #[test]
    fn apply_queue_mode_replace_clears_queue() {
        let (state, _root) = make_state_with_root();
        {
            let mut queue = state.playback.manager.queue_service().queue().lock().unwrap();
            queue.items.push(std::path::PathBuf::from("/music/a.flac"));
        }
        state
            .playback
            .manager
            .apply_queue_mode(std::path::Path::new("/music/b.flac"), QueueMode::Replace);
        let queue = state.playback.manager.queue_service().queue().lock().unwrap();
        assert!(queue.items.is_empty());
    }

    #[test]
    fn apply_queue_mode_append_adds_once() {
        let path = std::path::Path::new("/music/a.flac");
        let (state, _root) = make_state_with_root();
        state.playback.manager.apply_queue_mode(path, QueueMode::Append);
        state.playback.manager.apply_queue_mode(path, QueueMode::Append);
        let queue = state.playback.manager.queue_service().queue().lock().unwrap();
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
    fn play_request_sets_manual_advance_in_flight() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-play-request-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let library = crate::library::scan_library(&root).expect("scan library");
        let metadata_db = crate::metadata_db::MetadataDb::new(&root).unwrap();
        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id: Some("bridge:test:device".to_string()),
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
        let status = StatusStore::new(
            Arc::new(Mutex::new(crate::state::PlayerStatus::default())),
            crate::events::EventBus::new(),
        );
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(
            queue,
            status.clone(),
            crate::events::EventBus::new(),
        );
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status,
            queue_service,
        );
        let device_selection = crate::state::DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };
        let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
        let state = AppState::new(
            library,
            metadata_db,
            None,
            crate::state::MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            playback_manager,
            device_selection,
            crate::events::EventBus::new(),
            Arc::new(crate::events::LogBus::new(64)),
        );
        let provider = MockProvider {
            active_output_id: "bridge:test:device".to_string(),
            should_connect: true,
        };
        let controller = OutputController::new(OutputRegistry::new(vec![Box::new(provider)]));

        let path = root.join("track.flac");
        std::fs::write(&path, b"stub").unwrap();

        let result = actix_web::rt::System::new().block_on(async {
            controller
                .play_request(&state, path, QueueMode::Keep, None)
                .await
        });
        assert!(result.is_ok());

        let guard = state.playback.manager.status().inner().lock().unwrap();
        assert!(guard.manual_advance_in_flight);
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
    fn canonicalize_under_root_rejects_missing_path() {
        let (state, _root) = make_state_with_root();
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let result = controller.canonicalize_under_root(
            &state,
            std::path::Path::new("missing.flac"),
        );

        assert!(matches!(result, Err(OutputControllerError::Http(_))));
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
    fn queue_play_from_returns_false_when_missing() {
        let (state, root) = make_state_with_root();
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));
        let file_path = root.join("track.flac");
        let _ = std::fs::write(&file_path, b"test");

        let result = actix_web::rt::System::new().block_on(async {
            controller
                .queue_play_from(&state, "track.flac")
                .await
        });

        assert!(matches!(result, Ok(false)));
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
            state.playback.manager.queue_service().queue().lock().unwrap().items,
            vec![file_path.canonicalize().unwrap()]
        );
    }

    #[test]
    fn queue_remove_path_returns_false_when_not_queued() {
        let (state, root) = make_state_with_root();
        let file_path = root.join("a.flac");
        let _ = std::fs::write(&file_path, b"test");
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let removed = controller
            .queue_remove_path(&state, "a.flac")
            .expect("remove path");

        assert!(!removed);
    }

    #[test]
    fn queue_remove_path_removes_existing_item() {
        let (state, root) = make_state_with_root();
        let file_path = root.join("a.flac");
        let _ = std::fs::write(&file_path, b"test");
        let controller = OutputController::new(OutputRegistry::new(Vec::new()));

        let added = controller.queue_add_paths(&state, vec!["a.flac".to_string()]);
        assert_eq!(added, 1);
        let removed = controller
            .queue_remove_path(&state, "a.flac")
            .expect("remove path");

        assert!(removed);
        assert!(state.playback.manager
            .queue_service()
            .queue()
            .lock()
            .unwrap()
            .items
            .is_empty());
    }
}
