#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    pub chunk_frames: usize,
    pub refill_max_frames: usize,
    pub buffer_seconds: f32,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            chunk_frames: 1024,
            refill_max_frames: 4096,
            buffer_seconds: 2.0,
        }
    }
}
