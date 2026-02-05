//! Playback coordination for the active output.
//!
//! Wraps the active player handle, status store, and queue service for dispatch.

use std::sync::{Arc, Mutex};

use crate::playback_transport::{ChannelTransport, PlaybackTransport};
use crate::queue_service::{NextDispatchResult, QueueService};
use crate::bridge::BridgePlayer;
use crate::status_store::StatusStore;

#[derive(Clone)]
pub struct PlaybackManager {
    player: Arc<Mutex<BridgePlayer>>,
    status: StatusStore,
    queue_service: QueueService,
}

impl PlaybackManager {
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

    pub fn status(&self) -> &StatusStore {
        &self.status
    }

    pub fn queue_service(&self) -> &QueueService {
        &self.queue_service
    }

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
        self.status.on_play(path, start_paused);
        Ok(())
    }

    pub fn pause_toggle(&self) -> Result<(), ()> {
        let transport = self.transport();
        transport.pause_toggle().map_err(|_| ())?;
        self.status.on_pause_toggle();
        Ok(())
    }

    pub fn seek(&self, ms: u64) -> Result<(), ()> {
        let transport = self.transport();
        transport.seek(ms).map_err(|_| ())?;
        self.status.mark_seek_in_flight();
        Ok(())
    }

    pub fn stop(&self) -> Result<(), ()> {
        let transport = self.transport();
        transport.stop().map_err(|_| ())?;
        self.status.on_stop();
        Ok(())
    }

    pub fn queue_next(&self) -> NextDispatchResult {
        let transport = self.transport();
        self.queue_service.dispatch_next(&transport, false)
    }

    fn transport(&self) -> ChannelTransport {
        ChannelTransport::new(self.player.lock().unwrap().cmd_tx.clone())
    }
}
