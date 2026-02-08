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
        api::list_library,
        api::rescan_library,
        api::rescan_track,
        api::play_track,
        api::pause_toggle,
        api::seek,
        api::stream_track,
        api::queue_list,
        api::queue_add,
        api::queue_add_next,
        api::queue_remove,
        api::queue_clear,
        api::queue_play_from,
        api::queue_next,
        api::queue_stream,
        api::artists_list,
        api::albums_list,
        api::tracks_list,
        api::tracks_resolve,
        api::tracks_metadata,
        api::tracks_metadata_update,
        api::albums_metadata,
        api::albums_metadata_update,
        api::musicbrainz_match_search,
        api::musicbrainz_match_apply,
        api::art_for_track,
        api::track_cover,
        api::album_cover,
        api::logs_clear,
        api::status_for_output,
        api::status_stream,
        api::providers_list,
        api::provider_outputs_list,
        api::outputs_list,
        api::outputs_stream,
        api::metadata_stream,
        api::albums_stream,
        api::logs_stream,
        api::outputs_select,
    ),
    components(
        schemas(
            models::LibraryEntry,
            models::LibraryResponse,
            models::PlayRequest,
            models::QueueMode,
            api::RescanTrackRequest,
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
            models::MusicBrainzMatchSearchRequest,
            models::MusicBrainzMatchSearchResponse,
            models::MusicBrainzMatchCandidate,
            models::MusicBrainzMatchApplyRequest,
            models::MusicBrainzMatchKind,
            crate::metadata_db::ArtistSummary,
            crate::metadata_db::AlbumSummary,
            crate::metadata_db::TrackSummary,
            api::SeekBody,
            api::ArtQuery,
            api::CoverPath,
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
