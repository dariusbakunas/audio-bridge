//! Shared playback status store and update helpers.
//!
//! Centralizes state updates from local playback and bridge status polling.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use audio_bridge_types::BridgeStatus;

use crate::queue_service::AutoAdvanceInputs;
use crate::state::PlayerStatus;

#[derive(Clone)]
pub struct StatusStore {
    inner: Arc<Mutex<PlayerStatus>>,
}

impl StatusStore {
    pub fn new(inner: Arc<Mutex<PlayerStatus>>) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &Arc<Mutex<PlayerStatus>> {
        &self.inner
    }

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
    }

    pub fn on_pause_toggle(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.paused = !s.paused;
            s.user_paused = s.paused;
        }
    }

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
        }
    }

    pub fn mark_seek_in_flight(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.seek_in_flight = true;
        }
    }

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
    }

    pub fn on_local_playback_end(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.now_playing = None;
            s.elapsed_ms = None;
            s.duration_ms = None;
        }
    }

    pub fn set_auto_advance_in_flight(&self, value: bool) {
        if let Ok(mut s) = self.inner.lock() {
            s.auto_advance_in_flight = value;
        }
    }

    pub fn apply_remote_and_inputs(
        &self,
        remote: &BridgeStatus,
        last_duration_ms: Option<u64>,
    ) -> AutoAdvanceInputs {
        if let Ok(mut s) = self.inner.lock() {
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

            if s.seek_in_flight {
                if s.elapsed_ms.is_some() && s.duration_ms.is_some() {
                    s.seek_in_flight = false;
                }
            }

            return AutoAdvanceInputs {
                last_duration_ms,
                remote_duration_ms: remote.duration_ms,
                remote_elapsed_ms: remote.elapsed_ms,
                elapsed_ms: s.elapsed_ms,
                duration_ms: s.duration_ms,
                user_paused: s.user_paused,
                seek_in_flight: s.seek_in_flight,
                auto_advance_in_flight: s.auto_advance_in_flight,
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
