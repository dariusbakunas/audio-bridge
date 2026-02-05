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
use crate::playback_transport::ChannelTransport;
use crate::queue_service::QueueService;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Play {
        path: PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    },
    PauseToggle,
    Stop,
    Seek { ms: u64 },
    Quit,
}

#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

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
                        let title = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string());
                        let _ = client.play_path(
                            &path,
                            if ext_hint.is_empty() { None } else { Some(ext_hint.as_str()) },
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

struct BridgeStatusPoller {
    bridge_id: String,
    client: BridgeTransportClient,
    status: crate::status_store::StatusStore,
    queue: Arc<Mutex<crate::state::QueueState>>,
    cmd_tx: Sender<BridgeCommand>,
    bridge_online: Arc<AtomicBool>,
    bridges_state: Arc<Mutex<crate::state::BridgeState>>,
}

impl BridgeStatusPoller {
    fn new(
        bridge_id: String,
        client: BridgeTransportClient,
        status: crate::status_store::StatusStore,
        queue: Arc<Mutex<crate::state::QueueState>>,
        cmd_tx: Sender<BridgeCommand>,
        bridge_online: Arc<AtomicBool>,
        bridges_state: Arc<Mutex<crate::state::BridgeState>>,
    ) -> Self {
        Self {
            bridge_id,
            client,
            status,
            queue,
            cmd_tx,
            bridge_online,
            bridges_state,
        }
    }

    fn poll_once(&mut self, last_duration_ms: &mut Option<u64>) {
        match self.client.status() {
            Ok(remote) => {
                self.bridge_online.store(true, Ordering::Relaxed);
                let inputs = self.status.apply_remote_and_inputs(&remote, *last_duration_ms);
                let transport = ChannelTransport::new(self.cmd_tx.clone());
                let dispatched = QueueService::new(self.queue.clone(), self.status.clone())
                    .maybe_auto_advance(&transport, inputs);
                *last_duration_ms = remote.duration_ms;
                if dispatched {
                    return;
                }
            }
            Err(_) => {
                if self
                    .bridges_state
                    .lock()
                    .map(|s| s.active_bridge_id.as_deref() == Some(self.bridge_id.as_str()))
                    .unwrap_or(false)
                {
                    self.bridge_online.store(false, Ordering::Relaxed);
                }
            }
        }
    }
}
