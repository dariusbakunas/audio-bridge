//! Playback coordination for the active output.
//!
//! Wraps the active player handle, status store, and queue service for dispatch.

use std::sync::{Arc, Mutex};

use crate::playback_transport::{ChannelTransport, PlaybackTransport};
use crate::queue_service::{NextDispatchResult, QueueService};
use crate::bridge::BridgePlayer;
use crate::status_store::StatusStore;

/// Coordinates playback commands and status updates for the active output.
#[derive(Clone)]
pub struct PlaybackManager {
    player: Arc<Mutex<BridgePlayer>>,
    status: StatusStore,
    queue_service: QueueService,
}

impl PlaybackManager {
    /// Create a new manager for the active player.
    pub fn new(
        player: Arc<Mutex<BridgePlayer>>,
        status: StatusStore,
        queue_service: QueueService,
    ) -> Self {
        Self {
            player,
            status,
            queue_service,
        }
    }

    /// Access the shared status store.
    pub fn status(&self) -> &StatusStore {
        &self.status
    }

    /// Access the queue service.
    pub fn queue_service(&self) -> &QueueService {
        &self.queue_service
    }

    /// Start playback for a media path.
    pub fn play(
        &self,
        path: std::path::PathBuf,
        ext_hint: String,
        seek_ms: Option<u64>,
        start_paused: bool,
    ) -> Result<(), ()> {
        let transport = self.transport();
        transport
            .play(path.clone(), ext_hint, seek_ms, start_paused)
            .map_err(|_| ())?;
        self.status.on_play(path.clone(), start_paused);
        self.queue_service.record_played_path(&path);
        self.update_has_previous();
        Ok(())
    }

    /// Toggle pause state.
    pub fn pause_toggle(&self) -> Result<(), ()> {
        let transport = self.transport();
        transport.pause_toggle().map_err(|_| ())
    }

    /// Seek to the requested time in milliseconds.
    pub fn seek(&self, ms: u64) -> Result<(), ()> {
        let transport = self.transport();
        transport.seek(ms).map_err(|_| ())?;
        self.status.mark_seek_in_flight();
        Ok(())
    }

    /// Stop playback and reset status fields.
    pub fn stop(&self) -> Result<(), ()> {
        let transport = self.transport();
        transport.stop().map_err(|_| ())?;
        self.status.on_stop();
        Ok(())
    }

    /// Dispatch the next queue entry to the active transport.
    pub fn queue_next(&self) -> NextDispatchResult {
        let transport = self.transport();
        let result = self.queue_service.dispatch_next(&transport, false);
        self.update_has_previous();
        result
    }

    pub fn update_has_previous(&self) {
        let current = self
            .status
            .inner()
            .lock()
            .ok()
            .and_then(|guard| guard.now_playing.clone());
        let has_previous = self
            .queue_service
            .has_previous(current.as_deref());
        self.status.set_has_previous(has_previous);
    }

    fn transport(&self) -> ChannelTransport {
        ChannelTransport::new(self.player.lock().unwrap().cmd_tx.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use tokio::sync::broadcast::error::TryRecvError;
    use crate::bridge::BridgeCommand;
    use crate::state::PlayerStatus;

    #[test]
    fn pause_toggle_does_not_mutate_status_optimistically() {
        let (cmd_tx, cmd_rx) = unbounded();
        let player = Arc::new(Mutex::new(BridgePlayer { cmd_tx }));
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let events = crate::events::EventBus::new();
        let mut receiver = events.subscribe();
        let status_store = StatusStore::new(status.clone(), events.clone());
        let queue = Arc::new(Mutex::new(crate::state::QueueState::default()));
        let queue_service = QueueService::new(queue, status_store.clone(), events);
        let manager = PlaybackManager::new(player, status_store, queue_service);

        {
            let mut guard = status.lock().unwrap();
            guard.paused = true;
        }

        manager.pause_toggle().unwrap();

        assert!(matches!(cmd_rx.try_recv(), Ok(BridgeCommand::PauseToggle)));
        assert!(status.lock().unwrap().paused);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    }
}
