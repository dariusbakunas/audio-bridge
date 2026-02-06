//! Shared playback status store and update helpers.
//!
//! Centralizes state updates from local playback and bridge status polling.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use audio_bridge_types::BridgeStatus;

use crate::queue_service::AutoAdvanceInputs;
use crate::events::EventBus;
use crate::state::PlayerStatus;

/// Shared playback status tracker and update entry points.
#[derive(Clone)]
pub struct StatusStore {
    inner: Arc<Mutex<PlayerStatus>>,
    events: EventBus,
}

impl StatusStore {
    /// Create a new store around the shared player status.
    pub fn new(inner: Arc<Mutex<PlayerStatus>>, events: EventBus) -> Self {
        Self { inner, events }
    }

    /// Access the underlying shared status handle.
    pub fn inner(&self) -> &Arc<Mutex<PlayerStatus>> {
        &self.inner
    }

    /// Record a play request for the given media path.
    pub fn on_play(&self, path: PathBuf, start_paused: bool) {
        if let Ok(mut s) = self.inner.lock() {
            s.now_playing = Some(path);
            s.elapsed_ms = Some(0);
            s.user_paused = start_paused;
            apply_playback_fields(
                &mut s,
                PlaybackFields {
                    output_device: None,
                    sample_rate: None,
                    channels: None,
                    duration_ms: None,
                    source_codec: None,
                    source_bit_depth: None,
                    container: None,
                    output_sample_format: None,
                    resampling: None,
                    resample_from_hz: None,
                    resample_to_hz: None,
                    buffer_size_frames: None,
                    buffered_frames: None,
                    buffer_capacity_frames: None,
                    elapsed_ms: Some(0),
                    paused: Some(start_paused),
                },
            );
            s.auto_advance_in_flight = false;
            s.seek_in_flight = false;
        }
        self.events.status_changed();
    }

    pub fn on_pause_toggle(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.paused = !s.paused;
            s.user_paused = s.paused;
        }
        self.events.status_changed();
    }

    /// Clear status when playback stops.
    pub fn on_stop(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.now_playing = None;
            s.paused = false;
            s.user_paused = false;
            s.elapsed_ms = None;
            s.duration_ms = None;
            s.sample_rate = None;
            s.channels = None;
            s.source_codec = None;
            s.source_bit_depth = None;
            s.container = None;
            s.output_sample_format = None;
            s.resampling = None;
            s.resample_from_hz = None;
            s.resample_to_hz = None;
            s.buffer_size_frames = None;
            s.buffered_frames = None;
            s.buffer_capacity_frames = None;
            s.auto_advance_in_flight = false;
            s.seek_in_flight = false;
            s.manual_advance_in_flight = false;
        }
        self.events.status_changed();
    }

    pub fn mark_seek_in_flight(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.seek_in_flight = true;
        }
        self.events.status_changed();
    }

    /// Apply a local playback start snapshot.
    pub fn on_local_playback_start(
        &self,
        path: PathBuf,
        output_device: Option<String>,
        sample_rate: u32,
        channels: u16,
        duration_ms: Option<u64>,
        source_codec: Option<String>,
        source_bit_depth: Option<u16>,
        container: Option<String>,
        output_sample_format: Option<String>,
        resampling: bool,
        resample_from_hz: u32,
        resample_to_hz: u32,
        elapsed_ms: Option<u64>,
        paused: bool,
    ) {
        if let Ok(mut s) = self.inner.lock() {
            s.now_playing = Some(path);
            s.manual_advance_in_flight = false;
            apply_playback_fields(
                &mut s,
                PlaybackFields {
                    output_device,
                    sample_rate: Some(sample_rate),
                    channels: Some(channels),
                    duration_ms,
                    source_codec,
                    source_bit_depth,
                    container,
                    output_sample_format,
                    resampling: Some(resampling),
                    resample_from_hz: Some(resample_from_hz),
                    resample_to_hz: Some(resample_to_hz),
                    buffer_size_frames: None,
                    buffered_frames: None,
                    buffer_capacity_frames: None,
                    elapsed_ms,
                    paused: Some(paused),
                },
            );
        }
        self.events.status_changed();
    }

    pub fn on_local_playback_end(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.now_playing = None;
            s.elapsed_ms = None;
            s.duration_ms = None;
            s.manual_advance_in_flight = false;
        }
        self.events.status_changed();
    }

    pub fn set_manual_advance_in_flight(&self, value: bool) {
        if let Ok(mut s) = self.inner.lock() {
            s.manual_advance_in_flight = value;
        }
        self.events.status_changed();
    }

    /// Set whether auto-advance has been triggered and awaiting completion.
    pub fn set_auto_advance_in_flight(&self, value: bool) {
        if let Ok(mut s) = self.inner.lock() {
            s.auto_advance_in_flight = value;
        }
        self.events.status_changed();
    }

    /// Merge remote bridge status and return inputs for auto-advance checks.
    pub fn apply_remote_and_inputs(
        &self,
        remote: &BridgeStatus,
        last_duration_ms: Option<u64>,
    ) -> AutoAdvanceInputs {
        if let Ok(mut s) = self.inner.lock() {
            let prior_paused = s.paused;
            if prior_paused != remote.paused {
                tracing::debug!(
                    prior_paused,
                    paused = remote.paused,
                    "bridge status update"
                );
            }
            if should_clear_now_playing(&s, remote) {
                s.now_playing = None;
                s.paused = false;
                s.user_paused = false;
                s.auto_advance_in_flight = false;
            }
            apply_playback_fields(
                &mut s,
                PlaybackFields {
                    output_device: remote.device.clone(),
                    sample_rate: remote.sample_rate,
                    channels: remote.channels,
                    duration_ms: remote.duration_ms,
                    source_codec: remote.source_codec.clone(),
                    source_bit_depth: remote.source_bit_depth,
                    container: remote.container.clone(),
                    output_sample_format: remote.output_sample_format.clone(),
                    resampling: remote.resampling,
                    resample_from_hz: remote.resample_from_hz,
                    resample_to_hz: remote.resample_to_hz,
                    buffer_size_frames: remote.buffer_size_frames,
                    buffered_frames: remote.buffered_frames,
                    buffer_capacity_frames: remote.buffer_capacity_frames,
                    elapsed_ms: remote.elapsed_ms,
                    paused: Some(remote.paused),
                },
            );

            if should_clear_seek_in_flight(&s) {
                s.seek_in_flight = false;
            }
            if should_clear_manual_advance_in_flight(&s) {
                s.manual_advance_in_flight = false;
            }

            self.events.status_changed();
            return AutoAdvanceInputs {
                last_duration_ms,
                remote_duration_ms: remote.duration_ms,
                remote_elapsed_ms: remote.elapsed_ms,
                elapsed_ms: s.elapsed_ms,
                duration_ms: s.duration_ms,
                user_paused: s.user_paused,
                seek_in_flight: s.seek_in_flight,
                auto_advance_in_flight: s.auto_advance_in_flight,
                manual_advance_in_flight: s.manual_advance_in_flight,
                now_playing: s.now_playing.is_some(),
            };
        }

        AutoAdvanceInputs {
            last_duration_ms,
            remote_duration_ms: remote.duration_ms,
            remote_elapsed_ms: remote.elapsed_ms,
            elapsed_ms: None,
            duration_ms: None,
            user_paused: false,
            seek_in_flight: false,
            auto_advance_in_flight: false,
            manual_advance_in_flight: false,
            now_playing: false,
        }
    }
}

struct PlaybackFields {
    output_device: Option<String>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    duration_ms: Option<u64>,
    source_codec: Option<String>,
    source_bit_depth: Option<u16>,
    container: Option<String>,
    output_sample_format: Option<String>,
    resampling: Option<bool>,
    resample_from_hz: Option<u32>,
    resample_to_hz: Option<u32>,
    buffer_size_frames: Option<u32>,
    buffered_frames: Option<u64>,
    buffer_capacity_frames: Option<u64>,
    elapsed_ms: Option<u64>,
    paused: Option<bool>,
}

fn apply_playback_fields(s: &mut PlayerStatus, fields: PlaybackFields) {
    s.output_device = fields.output_device;
    s.sample_rate = fields.sample_rate;
    s.channels = fields.channels;
    s.duration_ms = fields.duration_ms;
    s.source_codec = fields.source_codec;
    s.source_bit_depth = fields.source_bit_depth;
    s.container = fields.container;
    s.output_sample_format = fields.output_sample_format;
    s.resampling = fields.resampling;
    s.resample_from_hz = fields.resample_from_hz;
    s.resample_to_hz = fields.resample_to_hz;
    s.buffer_size_frames = fields.buffer_size_frames;
    s.buffered_frames = fields.buffered_frames;
    s.buffer_capacity_frames = fields.buffer_capacity_frames;
    s.elapsed_ms = fields.elapsed_ms;
    if let Some(paused) = fields.paused {
        s.paused = paused;
    }
}

fn should_clear_seek_in_flight(state: &PlayerStatus) -> bool {
    state.seek_in_flight && state.elapsed_ms.is_some() && state.duration_ms.is_some()
}

fn should_clear_manual_advance_in_flight(state: &PlayerStatus) -> bool {
    state.manual_advance_in_flight && state.elapsed_ms.is_some() && state.duration_ms.is_some()
}

fn should_clear_now_playing(state: &PlayerStatus, remote: &BridgeStatus) -> bool {
    state.now_playing.is_some()
        && remote.now_playing.is_none()
        && remote.elapsed_ms.is_none()
        && remote.duration_ms.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn make_store() -> StatusStore {
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        StatusStore::new(status, crate::events::EventBus::new())
    }

    fn make_bridge_status() -> BridgeStatus {
        BridgeStatus {
            now_playing: None,
            paused: false,
            elapsed_ms: None,
            duration_ms: None,
            source_codec: None,
            source_bit_depth: None,
            container: None,
            output_sample_format: None,
            resampling: None,
            resample_from_hz: None,
            resample_to_hz: None,
            sample_rate: None,
            channels: None,
            device: None,
            underrun_frames: None,
            underrun_events: None,
            buffer_size_frames: None,
            buffered_frames: None,
            buffer_capacity_frames: None,
        }
    }

    #[test]
    fn on_play_sets_core_fields() {
        let store = make_store();
        store.on_play(PathBuf::from("/music/a.flac"), true);
        let status = store.inner().lock().unwrap();
        assert_eq!(status.now_playing, Some(PathBuf::from("/music/a.flac")));
        assert_eq!(status.elapsed_ms, Some(0));
        assert!(status.user_paused);
        assert!(status.paused);
    }

    #[test]
    fn on_play_does_not_clear_manual_advance_in_flight() {
        let store = make_store();
        if let Ok(mut status) = store.inner().lock() {
            status.manual_advance_in_flight = true;
        }
        store.on_play(PathBuf::from("/music/a.flac"), false);
        let status = store.inner().lock().unwrap();
        assert!(status.manual_advance_in_flight);
    }

    #[test]
    fn on_stop_clears_playback_fields() {
        let store = make_store();
        store.on_play(PathBuf::from("/music/a.flac"), false);
        store.on_stop();
        let status = store.inner().lock().unwrap();
        assert!(status.now_playing.is_none());
        assert!(status.duration_ms.is_none());
        assert!(!status.paused);
        assert!(!status.user_paused);
    }

    #[test]
    fn on_pause_toggle_flips_and_tracks_user_pause() {
        let store = make_store();
        store.on_play(PathBuf::from("/music/a.flac"), false);
        store.on_pause_toggle();
        let status = store.inner().lock().unwrap();
        assert!(status.paused);
        assert!(status.user_paused);
    }

    #[test]
    fn on_local_playback_start_sets_fields() {
        let store = make_store();
        store.on_local_playback_start(
            PathBuf::from("/music/a.flac"),
            Some("Device".to_string()),
            48000,
            2,
            Some(1000),
            Some("FLAC".to_string()),
            Some(24),
            Some("FLAC".to_string()),
            Some("F32".to_string()),
            true,
            96000,
            48000,
            Some(10),
            false,
        );
        let status = store.inner().lock().unwrap();
        assert_eq!(status.output_device.as_deref(), Some("Device"));
        assert_eq!(status.sample_rate, Some(48000));
        assert_eq!(status.channels, Some(2));
        assert_eq!(status.duration_ms, Some(1000));
        assert_eq!(status.source_codec.as_deref(), Some("FLAC"));
        assert_eq!(status.source_bit_depth, Some(24));
        assert_eq!(status.output_sample_format.as_deref(), Some("F32"));
        assert_eq!(status.resampling, Some(true));
        assert_eq!(status.resample_from_hz, Some(96000));
        assert_eq!(status.resample_to_hz, Some(48000));
        assert_eq!(status.elapsed_ms, Some(10));
        assert!(!status.paused);
    }

    #[test]
    fn should_clear_seek_in_flight_requires_elapsed_and_duration() {
        let mut status = PlayerStatus::default();
        status.seek_in_flight = true;
        assert!(!should_clear_seek_in_flight(&status));
        status.elapsed_ms = Some(10);
        assert!(!should_clear_seek_in_flight(&status));
        status.duration_ms = Some(100);
        assert!(should_clear_seek_in_flight(&status));
    }

    #[test]
    fn apply_remote_and_inputs_clears_seek_when_ready() {
        let store = make_store();
        store.mark_seek_in_flight();
        let remote = BridgeStatus {
            elapsed_ms: Some(10),
            duration_ms: Some(100),
            ..make_bridge_status()
        };

        store.apply_remote_and_inputs(&remote, None);
        let status = store.inner().lock().unwrap();
        assert!(!status.seek_in_flight);
    }

    #[test]
    fn apply_remote_and_inputs_clears_manual_advance_when_ready() {
        let store = make_store();
        store.set_manual_advance_in_flight(true);
        let remote = BridgeStatus {
            elapsed_ms: Some(10),
            duration_ms: Some(100),
            ..make_bridge_status()
        };

        store.apply_remote_and_inputs(&remote, None);
        let status = store.inner().lock().unwrap();
        assert!(!status.manual_advance_in_flight);
    }

    #[test]
    fn apply_remote_and_inputs_returns_auto_advance_inputs() {
        let store = make_store();
        let remote = BridgeStatus {
            elapsed_ms: Some(50),
            duration_ms: Some(100),
            ..make_bridge_status()
        };

        let inputs = store.apply_remote_and_inputs(&remote, Some(90));
        assert_eq!(inputs.last_duration_ms, Some(90));
        assert_eq!(inputs.remote_duration_ms, Some(100));
        assert_eq!(inputs.remote_elapsed_ms, Some(50));
        assert_eq!(inputs.elapsed_ms, Some(50));
        assert_eq!(inputs.duration_ms, Some(100));
        assert!(!inputs.user_paused);
        assert!(!inputs.seek_in_flight);
    }

    #[test]
    fn apply_remote_and_inputs_clears_now_playing_on_stop() {
        let store = make_store();
        store.on_play(PathBuf::from("/music/a.flac"), false);
        let remote = BridgeStatus {
            now_playing: None,
            elapsed_ms: None,
            duration_ms: None,
            ..make_bridge_status()
        };

        store.apply_remote_and_inputs(&remote, None);
        let status = store.inner().lock().unwrap();
        assert!(status.now_playing.is_none());
        assert!(!status.paused);
        assert!(!status.user_paused);
    }
}
