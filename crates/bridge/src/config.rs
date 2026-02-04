use std::net::SocketAddr;
use std::path::PathBuf;

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

#[derive(Clone, Debug)]
pub struct BridgeListenConfig {
    pub http_bind: SocketAddr,
    pub device: Option<String>,
    pub playback: PlaybackConfig,
}

#[derive(Clone, Debug)]
pub struct BridgePlayConfig {
    pub path: PathBuf,
    pub device: Option<String>,
    pub playback: PlaybackConfig,
}
