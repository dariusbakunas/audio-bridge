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
        api::queue_list,
        api::queue_add,
        api::queue_remove,
        api::queue_clear,
        api::queue_next,
        api::status,
        api::outputs_list,
        api::outputs_select,
        api::output_devices,
        api::output_set_device,
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
            models::OutputSelectRequest,
            models::BridgeDevicesResponse,
            models::BridgeSetDeviceRequest,
        )
    ),
    tags(
        (name = "audio-server", description = "Audio server control API")
    )
)]
pub struct ApiDoc;
