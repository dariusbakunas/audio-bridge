use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::Sender;

use crate::bridge::{BridgeCommand, BridgePlayer};
use crate::library::LibraryIndex;

#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    pub now_playing: Option<PathBuf>,
    pub paused: bool,
    pub duration_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub sample_rate: Option<u32>,
}

pub struct AppState {
    pub library: RwLock<LibraryIndex>,
    pub player: BridgePlayer,
    pub status: Arc<Mutex<PlayerStatus>>,
}

impl AppState {
    pub fn new(library: LibraryIndex, cmd_tx: Sender<BridgeCommand>, status: Arc<Mutex<PlayerStatus>>) -> Self {
        Self {
            library: RwLock::new(library),
            player: BridgePlayer { cmd_tx },
            status,
        }
    }
}
