use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use audio_bridge_types::{BridgeStatus as BridgeStatusSnapshot, PlaybackEndReason};

/// Shared playback status state updated by the player pipeline.
#[derive(Debug, Default)]
pub struct PlayerStatusState {
    /// Human-readable track identifier/path currently playing.
    pub now_playing: Option<String>,
    /// Selected output device name.
    pub device: Option<String>,
    /// Effective output sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Effective output channel count.
    pub channels: Option<u16>,
    /// Total track duration in milliseconds when known.
    pub duration_ms: Option<u64>,
    /// Source codec name (for example, FLAC/MP3).
    pub source_codec: Option<String>,
    /// Source bit depth when available.
    pub source_bit_depth: Option<u16>,
    /// Container format name.
    pub container: Option<String>,
    /// Output sample format used by the device stream.
    pub output_sample_format: Option<String>,
    /// Whether resampling is currently active.
    pub resampling: Option<bool>,
    /// Source sample rate before resampling.
    pub resample_from_hz: Option<u32>,
    /// Target sample rate after resampling.
    pub resample_to_hz: Option<u32>,
    /// Counter updated by playback callback for elapsed progress.
    pub played_frames: Option<Arc<AtomicU64>>,
    /// Pause state shared with playback callback.
    pub paused_flag: Option<Arc<AtomicBool>>,
    /// Total frames emitted as silence due to underruns.
    pub underrun_frames: Option<Arc<AtomicU64>>,
    /// Number of underrun incidents observed by callback.
    pub underrun_events: Option<Arc<AtomicU64>>,
    /// Configured output callback buffer size in frames.
    pub buffer_size_frames: Option<u32>,
    /// Current queued frames waiting for playback.
    pub buffered_frames: Option<Arc<AtomicU64>>,
    /// Max queue capacity in frames.
    pub buffer_capacity_frames: Option<Arc<AtomicU64>>,
    /// Terminal playback reason from the current run.
    pub end_reason: Option<PlaybackEndReason>,
}

/// Snapshot type returned to bridge HTTP/API layers.
pub type StatusSnapshot = BridgeStatusSnapshot;

impl PlayerStatusState {
    /// Create a shared, mutex-protected status store.
    pub fn shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }

    /// Return a snapshot suitable for API responses.
    pub fn snapshot(&self) -> StatusSnapshot {
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
            underrun_frames: self
                .underrun_frames
                .as_ref()
                .map(|v| v.load(Ordering::Relaxed)),
            underrun_events: self
                .underrun_events
                .as_ref()
                .map(|v| v.load(Ordering::Relaxed)),
            buffer_size_frames: self.buffer_size_frames,
            buffered_frames: self
                .buffered_frames
                .as_ref()
                .map(|v| v.load(Ordering::Relaxed)),
            buffer_capacity_frames: self
                .buffer_capacity_frames
                .as_ref()
                .map(|v| v.load(Ordering::Relaxed)),
            end_reason: self.end_reason,
        }
    }

    /// Clear track-specific fields when playback ends.
    pub fn clear_playback(&mut self) {
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
        self.buffered_frames = None;
        self.buffer_capacity_frames = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reports_elapsed_and_paused() {
        let mut state = PlayerStatusState::default();
        state.sample_rate = Some(48_000);
        state.played_frames = Some(Arc::new(AtomicU64::new(96_000)));
        state.paused_flag = Some(Arc::new(AtomicBool::new(true)));

        let snap = state.snapshot();
        assert_eq!(snap.elapsed_ms, Some(2000));
        assert!(snap.paused);
    }

    #[test]
    fn snapshot_includes_buffer_counters() {
        let mut state = PlayerStatusState::default();
        state.buffer_size_frames = Some(512);
        state.buffered_frames = Some(Arc::new(AtomicU64::new(1024)));
        state.buffer_capacity_frames = Some(Arc::new(AtomicU64::new(4096)));
        state.underrun_frames = Some(Arc::new(AtomicU64::new(12)));
        state.underrun_events = Some(Arc::new(AtomicU64::new(3)));

        let snap = state.snapshot();
        assert_eq!(snap.buffer_size_frames, Some(512));
        assert_eq!(snap.buffered_frames, Some(1024));
        assert_eq!(snap.buffer_capacity_frames, Some(4096));
        assert_eq!(snap.underrun_frames, Some(12));
        assert_eq!(snap.underrun_events, Some(3));
    }

    #[test]
    fn clear_playback_resets_track_fields() {
        let mut state = PlayerStatusState::default();
        state.now_playing = Some("track".to_string());
        state.sample_rate = Some(48_000);
        state.channels = Some(2);
        state.duration_ms = Some(10);
        state.source_codec = Some("FLAC".to_string());
        state.played_frames = Some(Arc::new(AtomicU64::new(1)));
        state.paused_flag = Some(Arc::new(AtomicBool::new(false)));
        state.buffered_frames = Some(Arc::new(AtomicU64::new(1)));
        state.end_reason = Some(PlaybackEndReason::Eof);

        state.clear_playback();

        assert!(state.now_playing.is_none());
        assert!(state.sample_rate.is_none());
        assert!(state.channels.is_none());
        assert!(state.duration_ms.is_none());
        assert!(state.source_codec.is_none());
        assert!(state.played_frames.is_none());
        assert!(state.paused_flag.is_none());
        assert!(state.buffered_frames.is_none());
        assert_eq!(state.end_reason, Some(PlaybackEndReason::Eof));
    }
}
