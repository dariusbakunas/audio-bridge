//! Playback transport abstraction for dispatching playback commands.
//!
//! Implementations translate playback operations into local or bridge commands.

use std::path::PathBuf;

use crossbeam_channel::Sender;

use crate::bridge::BridgeCommand;

/// Errors that can occur when dispatching playback commands.
#[derive(Debug)]
pub enum PlaybackTransportError {
    /// Transport is unavailable or disconnected.
    Offline,
}

/// Transport for playback commands implemented by local or bridge outputs.
pub trait PlaybackTransport {
    /// Start playback for a path with optional seek and pause.
    fn play(
        &self,
        path: PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<(), PlaybackTransportError>;
    /// Toggle pause/resume.
    fn pause_toggle(&self) -> Result<(), PlaybackTransportError>;
    /// Stop playback.
    fn stop(&self) -> Result<(), PlaybackTransportError>;
    /// Seek to an absolute position (milliseconds).
    fn seek(&self, ms: u64) -> Result<(), PlaybackTransportError>;
}

/// Playback transport that sends commands over a channel.
pub struct ChannelTransport {
    cmd_tx: Sender<BridgeCommand>,
}

impl ChannelTransport {
    /// Create a channel-backed transport.
    pub fn new(cmd_tx: Sender<BridgeCommand>) -> Self {
        Self { cmd_tx }
    }
}

impl PlaybackTransport for ChannelTransport {
    fn play(
        &self,
        path: PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<(), PlaybackTransportError> {
        self.cmd_tx
            .send(BridgeCommand::Play {
                path,
                ext_hint,
                seek_ms,
                start_paused,
            })
            .map_err(|_| PlaybackTransportError::Offline)
    }

    fn pause_toggle(&self) -> Result<(), PlaybackTransportError> {
        self.cmd_tx
            .send(BridgeCommand::PauseToggle)
            .map_err(|_| PlaybackTransportError::Offline)
    }

    fn stop(&self) -> Result<(), PlaybackTransportError> {
        self.cmd_tx
            .send(BridgeCommand::Stop)
            .map_err(|_| PlaybackTransportError::Offline)
    }

    fn seek(&self, ms: u64) -> Result<(), PlaybackTransportError> {
        self.cmd_tx
            .send(BridgeCommand::Seek { ms })
            .map_err(|_| PlaybackTransportError::Offline)
    }
}
