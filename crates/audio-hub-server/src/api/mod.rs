//! HTTP API handlers.
//!
//! Defines the Actix routes for library, playback, queue, and output control.

pub mod library;
pub mod logs;
pub mod local_playback;
pub mod metadata;
pub mod outputs;
pub mod sessions;
pub mod streams;
pub mod health;

pub use library::{
    list_library,
    rescan_library,
    rescan_track,
    stream_track_id,
    transcode_track_id,
};
pub use logs::{logs_clear, LogsClearResponse};
pub use local_playback::{
    local_playback_play,
    local_playback_register,
    local_playback_sessions,
};
pub use metadata::{
    album_image_clear,
    album_image_set,
    album_profile,
    album_profile_update,
    album_cover,
    albums_list,
    albums_metadata,
    albums_metadata_update,
    artist_image_clear,
    artist_image_set,
    artist_profile,
    artist_profile_update,
    artists_list,
    media_asset,
    musicbrainz_match_apply,
    musicbrainz_match_search,
    track_cover,
    tracks_list,
    tracks_metadata,
    tracks_metadata_fields,
    tracks_metadata_update,
    tracks_resolve,
    tracks_analysis,
};
pub use outputs::{
    outputs_list,
    outputs_select,
    outputs_settings,
    outputs_settings_update,
    provider_outputs_list,
    provider_refresh,
    providers_list,
};
pub use sessions::{
    sessions_create,
    sessions_delete,
    sessions_get,
    sessions_heartbeat,
    sessions_locks,
    sessions_list,
    sessions_mute_set,
    sessions_pause,
    sessions_queue_add,
    sessions_queue_add_next,
    sessions_queue_clear,
    sessions_queue_list,
    sessions_queue_next,
    sessions_queue_play_from,
    sessions_queue_previous,
    sessions_queue_remove,
    sessions_queue_stream,
    sessions_release_output,
    sessions_seek,
    sessions_select_output,
    sessions_status,
    sessions_status_stream,
    sessions_volume,
    sessions_volume_set,
    sessions_stop,
};
pub use streams::{
    albums_stream,
    logs_stream,
    metadata_stream,
    outputs_stream,
};
pub use health::HealthResponse;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use actix_web::{test, App};

    use crate::api;
    use crate::events::{EventBus, LogBus};
    use crate::state::{
        AppState, BridgeProviderState, BridgeState, DeviceSelectionState, LocalProviderState,
        MetadataWake, PlayerStatus, QueueState,
    };

    fn make_state() -> actix_web::web::Data<AppState> {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-server-api-smoke-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let library = crate::library::scan_library(&root).expect("scan library");
        let metadata_db = crate::metadata_db::MetadataDb::new(&root).expect("metadata db");

        let (cmd_tx, _cmd_rx) = crossbeam_channel::unbounded();
        let bridges_state = Arc::new(Mutex::new(BridgeState {
            bridges: Vec::new(),
            active_bridge_id: None,
            active_output_id: None,
        }));
        let bridge_state = Arc::new(BridgeProviderState::new(
            cmd_tx,
            bridges_state,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            "http://localhost".to_string(),
        ));

        let (local_cmd_tx, _local_cmd_rx) = crossbeam_channel::unbounded();
        let local_state = Arc::new(LocalProviderState {
            enabled: false,
            id: "local".to_string(),
            name: "Local Host".to_string(),
            player: Arc::new(Mutex::new(crate::bridge::BridgePlayer { cmd_tx: local_cmd_tx })),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });

        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let events = EventBus::new();
        let status_store = crate::status_store::StatusStore::new(status, events.clone());
        let queue = Arc::new(Mutex::new(QueueState::default()));
        let queue_service = crate::queue_service::QueueService::new(queue, status_store.clone(), events.clone());
        let playback_manager = crate::playback_manager::PlaybackManager::new(
            bridge_state.player.clone(),
            status_store,
            queue_service,
        );

        let device_selection = DeviceSelectionState {
            local: Arc::new(Mutex::new(None)),
            bridge: Arc::new(Mutex::new(std::collections::HashMap::new())),
        };

        let cast_state = Arc::new(crate::state::CastProviderState::new());
        let state = AppState::new(
            library,
            metadata_db,
            None,
            MetadataWake::new(),
            bridge_state,
            local_state,
            cast_state,
            playback_manager,
            device_selection,
            events,
            Arc::new(LogBus::new(64)),
            Arc::new(Mutex::new(crate::state::OutputSettingsState::default())),
            None,
        );

        actix_web::web::Data::new(state)
    }

    #[actix_web::test]
    async fn tracks_metadata_missing_returns_404() {
        let state = make_state();
        let app = test::init_service(App::new().app_data(state.clone()).service(api::tracks_metadata))
            .await;

        let req = test::TestRequest::get()
            .uri("/tracks/metadata?track_id=999999")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn library_list_root_ok() {
        let state = make_state();
        let app = test::init_service(App::new().app_data(state.clone()).service(api::list_library)).await;

        let req = test::TestRequest::get().uri("/library").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

}
