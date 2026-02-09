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

pub use library::{
    list_library,
    rescan_library,
    rescan_track,
    stream_track,
    LibraryQuery,
    RescanTrackRequest,
    StreamQuery,
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
    AlbumListQuery,
    AlbumMetadataQuery,
    ArtQuery,
    CoverPath,
    ListQuery,
    TrackListQuery,
    TrackMetadataQuery,
    TrackResolveQuery,
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
    SeekBody,
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
