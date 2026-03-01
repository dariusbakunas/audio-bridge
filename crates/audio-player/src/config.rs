/// Playback tuning parameters shared by decode/resample/playback stages.
#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    /// Decoder/resampler chunk size in frames.
    pub chunk_frames: usize,
    /// Max frames pulled per output callback refill.
    pub refill_max_frames: usize,
    /// Target buffer duration for queue sizing.
    pub buffer_seconds: f32,
}

impl Default for PlaybackConfig {
    /// Defaults tuned for low-risk playback across common devices.
    fn default() -> Self {
        Self {
            chunk_frames: 1024,
            refill_max_frames: 4096,
            buffer_seconds: 2.0,
        }
    }
}
