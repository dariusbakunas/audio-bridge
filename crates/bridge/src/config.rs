use std::net::SocketAddr;
use std::path::PathBuf;

/// Playback configuration shared with the audio-player crate.
pub use audio_player::config::PlaybackConfig;

/// Configuration for running the bridge HTTP listener.
#[derive(Clone, Debug)]
pub struct BridgeListenConfig {
    /// HTTP bind address for the bridge API.
    pub http_bind: SocketAddr,
    /// Optional output device name.
    pub device: Option<String>,
    /// Playback tuning options.
    pub playback: PlaybackConfig,
    /// Allow insecure TLS when streaming from the hub.
    pub tls_insecure: bool,
    /// Enable exclusive (hog) mode on macOS.
    pub exclusive_mode: bool,
}

/// Configuration for playing a local file once.
#[derive(Clone, Debug)]
pub struct BridgePlayConfig {
    /// Local file path to play.
    pub path: PathBuf,
    /// Optional output device name.
    pub device: Option<String>,
    /// Playback tuning options.
    pub playback: PlaybackConfig,
    /// Allow insecure TLS when streaming from the hub.
    pub tls_insecure: bool,
    /// Enable exclusive (hog) mode on macOS.
    pub exclusive_mode: bool,
}
