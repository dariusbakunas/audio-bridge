//! OpenAPI schema definition.
//!
//! Aggregates the API paths and model schemas for Swagger UI.

use utoipa::OpenApi;

use crate::api;
use crate::models;

/// OpenAPI document describing the hub server endpoints.
#[derive(OpenApi)]
#[openapi(
    paths(
        api::library::list_library,
        api::library::rescan_library,
        api::library::rescan_track,
        api::playback::play_track,
        api::playback::play_album,
        api::playback::pause_toggle,
        api::playback::seek,
        api::library::stream_track,
        api::library::transcode_track,
        api::queue::queue_list,
        api::queue::queue_add,
        api::queue::queue_add_next,
        api::queue::queue_remove,
        api::queue::queue_clear,
        api::queue::queue_play_from,
        api::queue::queue_next,
        api::queue::queue_previous,
        api::streams::queue_stream,
        api::metadata::artists_list,
        api::metadata::albums_list,
        api::metadata::tracks_list,
        api::metadata::tracks_resolve,
        api::metadata::tracks_metadata,
        api::metadata::tracks_metadata_update,
        api::metadata::albums_metadata,
        api::metadata::albums_metadata_update,
        api::metadata::musicbrainz_match_search,
        api::metadata::musicbrainz_match_apply,
        api::metadata::art_for_track,
        api::metadata::track_cover,
        api::metadata::album_cover,
        api::logs::logs_clear,
        api::playback::status_for_output,
        api::streams::status_stream,
        api::outputs::providers_list,
        api::outputs::provider_outputs_list,
        api::outputs::outputs_list,
        api::streams::outputs_stream,
        api::streams::metadata_stream,
        api::streams::albums_stream,
        api::streams::logs_stream,
        api::outputs::outputs_select,
    ),
    components(
        schemas(
            models::LibraryEntry,
            models::LibraryResponse,
            models::PlayRequest,
            models::PlayAlbumRequest,
            models::QueueMode,
            models::AlbumQueueMode,
            audio_bridge_types::PlaybackStatus,
            models::QueueItem,
            models::QueueResponse,
            models::QueueAddRequest,
            models::QueueRemoveRequest,
            models::QueuePlayFromRequest,
            models::OutputsResponse,
            models::OutputInfo,
            models::OutputCapabilities,
            models::SupportedRates,
            models::OutputSelectRequest,
            models::ProviderInfo,
            models::ProvidersResponse,
            models::ArtistListResponse,
            models::AlbumListResponse,
            models::TrackListResponse,
            models::TrackResolveResponse,
            models::TrackMetadataResponse,
            models::TrackMetadataUpdateRequest,
            models::AlbumMetadataResponse,
            models::AlbumMetadataUpdateRequest,
            models::AlbumMetadataUpdateResponse,
            models::MusicBrainzMatchSearchRequest,
            models::MusicBrainzMatchSearchResponse,
            models::MusicBrainzMatchCandidate,
            models::MusicBrainzMatchApplyRequest,
            models::MusicBrainzMatchKind,
            crate::metadata_db::ArtistSummary,
            crate::metadata_db::AlbumSummary,
            crate::metadata_db::TrackSummary,
            crate::events::MetadataEvent,
            crate::events::LogEvent,
            api::LogsClearResponse,
        )
    ),
    tags(
        (name = "audio-hub-server", description = "Audio server control API")
    )
)]
pub struct ApiDoc;
