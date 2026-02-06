//! Bridge worker + command dispatch loop.
//!
//! Owns the control channel for a single bridge and polls its status.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};

use crate::bridge_transport::BridgeTransportClient;
use crate::events::EventBus;
use crate::playback_transport::ChannelTransport;
use crate::queue_service::QueueService;

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
    cmd_tx: Sender<BridgeCommand>,
    status: crate::status_store::StatusStore,
    queue: Arc<Mutex<crate::state::QueueState>>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
    public_base_url: String,
    events: EventBus,
) {
    std::thread::spawn(move || {
        tracing::info!(bridge_id = %bridge_id, http_addr = %http_addr, "bridge worker start");
        let mut next_poll = Instant::now();
        let mut last_duration_ms: Option<u64> = None;
        let client = BridgeTransportClient::new(http_addr, public_base_url);
        let mut poller = BridgeStatusPoller::new(
            bridge_id,
            client.clone(),
            status.clone(),
            queue.clone(),
            cmd_tx.clone(),
            bridge_online.clone(),
            bridges_state.clone(),
            events.clone(),
        );

        loop {
            let now = Instant::now();
            let timeout = next_poll.saturating_duration_since(now).min(Duration::from_millis(250));
            if let Ok(cmd) = cmd_rx.recv_timeout(timeout) {
                match cmd {
                    BridgeCommand::Quit => break,
                    BridgeCommand::PauseToggle => {
                        let _ = client.pause_toggle();
                        status.on_pause_toggle();
                    }
                    BridgeCommand::Stop => {
                        let _ = client.stop();
                        status.on_stop();
                    }
                    BridgeCommand::Seek { ms } => {
                        let _ = client.seek(ms);
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
                        );

                        status.on_play(path, false);
                    }
                }
            }

            if Instant::now() < next_poll {
                continue;
            }
            next_poll = Instant::now() + Duration::from_millis(500);
            poller.poll_once(&mut last_duration_ms);
        }
    });
}

/// Helper that polls bridge status and triggers auto-advance.
struct BridgeStatusPoller {
    bridge_id: String,
    client: BridgeTransportClient,
    status: crate::status_store::StatusStore,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
    events: EventBus,
}

impl BridgeStatusPoller {
    /// Create a status poller bound to a single bridge.
    fn new(
        bridge_id: String,
        client: BridgeTransportClient,
        status: crate::status_store::StatusStore,
        queue: Arc<Mutex<crate::state::QueueState>>,
        cmd_tx: Sender<BridgeCommand>,
        bridge_online: Arc<AtomicBool>,
        bridges_state: Arc<Mutex<crate::state::BridgeState>>,
        events: EventBus,
    ) -> Self {
        Self {
            bridge_id,
            client,
            status,
            queue,
            cmd_tx,
            bridge_online,
            bridges_state,
            events,
        }
    }

    /// Poll bridge status once and perform auto-advance if needed.
    fn poll_once(&mut self, last_duration_ms: &mut Option<u64>) {
        match self.client.status() {
            Ok(remote) => {
                self.bridge_online.store(true, Ordering::Relaxed);
                let inputs = self.status.apply_remote_and_inputs(&remote, *last_duration_ms);
                let transport = ChannelTransport::new(self.cmd_tx.clone());
                let dispatched = QueueService::new(
                    self.queue.clone(),
                    self.status.clone(),
                    self.events.clone(),
                )
                    .maybe_auto_advance(&transport, inputs);
                *last_duration_ms = remote.duration_ms;
                if dispatched {
                    return;
                }
            }
            Err(_) => {
                if is_active_bridge(&self.bridges_state, &self.bridge_id) {
                    self.bridge_online.store(false, Ordering::Relaxed);
                }
            }
        }
    }
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

fn is_active_bridge(
    bridges_state: &Arc<Mutex<crate::state::BridgeState>>,
    bridge_id: &str,
) -> bool {
    bridges_state
        .lock()
        .map(|s| s.active_bridge_id.as_deref() == Some(bridge_id))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
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
    fn is_active_bridge_matches_state() {
        let bridges = Arc::new(Mutex::new(crate::state::BridgeState {
            bridges: Vec::new(),
            active_bridge_id: Some("bridge-1".to_string()),
            active_output_id: None,
        }));
        assert!(is_active_bridge(&bridges, "bridge-1"));
        assert!(!is_active_bridge(&bridges, "bridge-2"));
    }
}
