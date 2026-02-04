use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use audio_bridge_types::BridgeStatus as BridgeStatusSnapshot;

#[derive(Debug, Default)]
pub(crate) struct BridgeStatusState {
    pub(crate) now_playing: Option<String>,
    pub(crate) device: Option<String>,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) channels: Option<u16>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) source_codec: Option<String>,
    pub(crate) source_bit_depth: Option<u16>,
    pub(crate) container: Option<String>,
    pub(crate) output_sample_format: Option<String>,
    pub(crate) resampling: Option<bool>,
    pub(crate) resample_from_hz: Option<u32>,
    pub(crate) resample_to_hz: Option<u32>,
    pub(crate) played_frames: Option<Arc<AtomicU64>>,
    pub(crate) paused_flag: Option<Arc<AtomicBool>>,
    pub(crate) underrun_frames: Option<Arc<AtomicU64>>,
    pub(crate) underrun_events: Option<Arc<AtomicU64>>,
    pub(crate) buffer_size_frames: Option<u32>,
}

pub(crate) type StatusSnapshot = BridgeStatusSnapshot;

impl BridgeStatusState {
    pub(crate) fn shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn snapshot(&self) -> StatusSnapshot {
        let paused = self
            .paused_flag
            .as_ref()
            .map(|p| p.load(Ordering::Relaxed))
            .unwrap_or(false);
        let elapsed_ms = match (self.played_frames.as_ref(), self.sample_rate) {
            (Some(frames), Some(sr)) if sr > 0 => {
                let frames = frames.load(Ordering::Relaxed);
                Some(frames.saturating_mul(1000) / sr as u64)
            }
            _ => None,
        };
        BridgeStatusSnapshot {
            now_playing: self.now_playing.clone(),
            paused,
            elapsed_ms,
            duration_ms: self.duration_ms,
            source_codec: self.source_codec.clone(),
            source_bit_depth: self.source_bit_depth,
            container: self.container.clone(),
            output_sample_format: self.output_sample_format.clone(),
            resampling: self.resampling,
            resample_from_hz: self.resample_from_hz,
            resample_to_hz: self.resample_to_hz,
            sample_rate: self.sample_rate,
            channels: self.channels,
            device: self.device.clone(),
            underrun_frames: self.underrun_frames.as_ref().map(|v| v.load(Ordering::Relaxed)),
            underrun_events: self.underrun_events.as_ref().map(|v| v.load(Ordering::Relaxed)),
            buffer_size_frames: self.buffer_size_frames,
        }
    }

    pub(crate) fn clear_playback(&mut self) {
        self.now_playing = None;
        self.sample_rate = None;
        self.channels = None;
        self.duration_ms = None;
        self.source_codec = None;
        self.source_bit_depth = None;
        self.container = None;
        self.output_sample_format = None;
        self.resampling = None;
        self.resample_from_hz = None;
        self.resample_to_hz = None;
        self.played_frames = None;
        self.paused_flag = None;
        self.underrun_frames = None;
        self.underrun_events = None;
        self.buffer_size_frames = None;
    }
}
