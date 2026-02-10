//! HTTP API handlers.
//!
//! Defines the Actix routes for library, playback, queue, and output control.

pub mod library;
pub mod logs;
pub mod metadata;
pub mod outputs;
pub mod playback;
pub mod queue;
pub mod streams;
pub mod browser;

pub use library::{
    list_library,
    rescan_library,
    rescan_track,
    stream_track,
    transcode_track,
};
pub use logs::{logs_clear, LogsClearResponse};
pub use metadata::{
    album_cover,
    albums_list,
    albums_metadata,
    albums_metadata_update,
    art_for_track,
    artists_list,
    musicbrainz_match_apply,
    musicbrainz_match_search,
    track_cover,
    tracks_list,
    tracks_metadata,
    tracks_metadata_update,
    tracks_resolve,
};
pub use outputs::{
    outputs_list,
    outputs_select,
    provider_outputs_list,
    providers_list,
};
pub use playback::{
    pause_toggle,
    play_track,
    seek,
    status_for_output,
    stop,
};
pub use queue::{
    queue_add,
    queue_add_next,
    queue_clear,
    queue_list,
    queue_next,
    queue_play_from,
    queue_previous,
    queue_remove,
};
pub use streams::{
    albums_stream,
    logs_stream,
    metadata_stream,
    outputs_stream,
    queue_stream,
    status_stream,
};
pub use browser::browser_ws;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use actix_web::{test, App};

    use crate::api;
    use crate::events::{EventBus, LogBus};
    use crate::models::{QueueAddRequest, QueueResponse};
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

        let browser_state = Arc::new(crate::browser::BrowserProviderState::new());
        let state = AppState::new(
            library,
            metadata_db,
            None,
            MetadataWake::new(),
            bridge_state,
            local_state,
            browser_state,
            playback_manager,
            device_selection,
            events,
            Arc::new(LogBus::new(64)),
        );

        actix_web::web::Data::new(state)
    }

    #[actix_web::test]
    async fn queue_add_and_list_round_trip() {
        let state = make_state();
        let file_path = state.library.read().unwrap().root().join("track.flac");
        std::fs::write(&file_path, b"stub").expect("write file");

        let app = test::init_service(
            App::new()
                .app_data(state.clone())
                .service(api::queue_add)
                .service(api::queue_list),
        )
        .await;

        let payload = QueueAddRequest {
            paths: vec!["track.flac".to_string()],
        };
        let req = test::TestRequest::post()
            .uri("/queue")
            .set_json(&payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let req = test::TestRequest::get().uri("/queue").to_request();
        let resp: QueueResponse = test::call_and_read_body_json(&app, req).await;
        assert_eq!(resp.items.len(), 1);
    }

    #[actix_web::test]
    async fn tracks_metadata_missing_returns_404() {
        let state = make_state();
        let app = test::init_service(App::new().app_data(state.clone()).service(api::tracks_metadata))
            .await;

        let req = test::TestRequest::get()
            .uri("/tracks/metadata?path=missing.flac")
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
