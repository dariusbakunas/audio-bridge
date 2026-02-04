use std::net::SocketAddr;
use std::path::PathBuf;

pub use audio_player::config::PlaybackConfig;

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
