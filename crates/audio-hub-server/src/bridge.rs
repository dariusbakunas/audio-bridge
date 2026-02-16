//! Bridge worker + command dispatch loop.
//!
//! Owns the control channel for a single bridge.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use crossbeam_channel::{Receiver, Sender};
use crossbeam_channel::TryRecvError;

use crate::bridge_transport::BridgeTransportClient;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    /// Start playback for a path, optionally seeking and pausing.
    Play {
        path: PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    },
    /// Toggle pause/resume.
    PauseToggle,
    /// Stop playback immediately.
    Stop,
    /// Seek to an absolute position (milliseconds).
    Seek { ms: u64 },
    /// Quit the bridge worker loop.
    Quit,
}

/// Handle to send bridge commands to the worker loop.
#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

/// Spawn the background bridge worker loop.
pub fn spawn_bridge_worker(
    bridge_id: String,
    http_addr: SocketAddr,
    cmd_rx: Receiver<BridgeCommand>,
    status: crate::status_store::StatusStore,
    worker_running: Arc<AtomicBool>,
    public_base_url: String,
    metadata: Option<crate::metadata_db::MetadataDb>,
) {
    actix_web::rt::spawn(async move {
        worker_running.store(true, Ordering::Relaxed);
        tracing::info!(bridge_id = %bridge_id, http_addr = %http_addr, "bridge worker start");
        let client = BridgeTransportClient::new_with_base(http_addr, public_base_url, metadata);

        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => match cmd {
                    BridgeCommand::Quit => break,
                    BridgeCommand::PauseToggle => {
                        let _ = client.pause_toggle().await;
                        status.on_pause_toggle();
                    }
                    BridgeCommand::Stop => {
                        let _ = client.stop().await;
                        status.on_stop();
                    }
                    BridgeCommand::Seek { ms } => {
                        let _ = client.seek(ms).await;
                        status.mark_seek_in_flight();
                    }
                    BridgeCommand::Play { path, ext_hint, seek_ms, start_paused } => {
                        let title = title_from_path(&path);
                        let _ = client.play_path(
                            &path,
                            ext_hint_option(&ext_hint),
                            title.as_deref(),
                            seek_ms,
                            start_paused,
                        )
                        .await;

                        status.on_play(path, false);
                    }
                },
                Err(TryRecvError::Empty) => {
                    actix_web::rt::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(TryRecvError::Disconnected) => break,
            }
        }
        worker_running.store(false, Ordering::Relaxed);
    });
}

fn title_from_path(path: &PathBuf) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn ext_hint_option(ext_hint: &str) -> Option<&str> {
    if ext_hint.trim().is_empty() {
        None
    } else {
        Some(ext_hint)
    }
}

pub(crate) fn update_online_and_should_emit(bridge_online: &AtomicBool, new_status: bool) -> bool {
    let was_online = bridge_online.swap(new_status, Ordering::Relaxed);
    was_online != new_status
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use super::*;

    #[test]
    fn title_from_path_returns_file_name() {
        let path = PathBuf::from("/music/track.flac");
        assert_eq!(title_from_path(&path), Some("track.flac".to_string()));
    }

    #[test]
    fn title_from_path_returns_none_for_dir() {
        let path = PathBuf::from("/");
        assert!(title_from_path(&path).is_none());
    }

    #[test]
    fn ext_hint_option_rejects_empty() {
        assert!(ext_hint_option("").is_none());
        assert!(ext_hint_option("   ").is_none());
        assert_eq!(ext_hint_option("flac"), Some("flac"));
    }

    #[test]
    fn update_online_and_should_emit_tracks_transitions() {
        let online = AtomicBool::new(false);
        assert!(update_online_and_should_emit(&online, true));
        assert!(!update_online_and_should_emit(&online, true));
        assert!(update_online_and_should_emit(&online, false));
        assert!(!update_online_and_should_emit(&online, false));
    }
}
