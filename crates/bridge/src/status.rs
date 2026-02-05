//! Status types shared with the audio-player crate.

/// Shared status state used by the bridge runtime.
pub(crate) use audio_player::status::PlayerStatusState as BridgeStatusState;
/// Snapshot payload returned by the HTTP status endpoint.
pub(crate) use audio_player::status::StatusSnapshot;
