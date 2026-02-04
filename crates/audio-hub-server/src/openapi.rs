use utoipa::OpenApi;

use crate::api;
use crate::models;

#[derive(OpenApi)]
#[openapi(
    paths(
        api::list_library,
        api::rescan_library,
        api::play_track,
        api::pause_toggle,
        api::seek,
        api::stream_track,
        api::queue_list,
        api::queue_add,
        api::queue_remove,
        api::queue_clear,
        api::queue_next,
        api::status_for_output,
        api::bridges_list,
        api::bridge_outputs_list,
        api::outputs_list,
        api::outputs_select,
    ),
    components(
        schemas(
            models::LibraryEntry,
            models::LibraryResponse,
            models::PlayRequest,
            models::QueueMode,
            models::StatusResponse,
            models::QueueItem,
            models::QueueResponse,
            models::QueueAddRequest,
            models::QueueRemoveRequest,
            models::OutputsResponse,
            models::OutputInfo,
            models::OutputCapabilities,
            models::SupportedRates,
            models::OutputSelectRequest,
            models::BridgeInfo,
            models::BridgesResponse,
            api::SeekBody,
        )
    ),
    tags(
        (name = "audio-hub-server", description = "Audio server control API")
    )
)]
pub struct ApiDoc;
