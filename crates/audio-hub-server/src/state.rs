use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::Sender;

use crate::bridge::{BridgeCommand, BridgePlayer};
use crate::library::LibraryIndex;

#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    pub now_playing: Option<PathBuf>,
    pub paused: bool,
    pub user_paused: bool,
    pub duration_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub auto_advance_in_flight: bool,
}

pub struct AppState {
    pub library: RwLock<LibraryIndex>,
    pub player: BridgePlayer,
    pub status: Arc<Mutex<PlayerStatus>>,
    pub queue: Arc<Mutex<QueueState>>,
    pub outputs: Arc<Mutex<OutputState>>,
}

impl AppState {
    pub fn new(
        library: LibraryIndex,
        cmd_tx: Sender<BridgeCommand>,
        status: Arc<Mutex<PlayerStatus>>,
        queue: Arc<Mutex<QueueState>>,
        outputs: Arc<Mutex<OutputState>>,
    ) -> Self {
        Self {
            library: RwLock::new(library),
            player: BridgePlayer { cmd_tx },
            status,
            queue,
            outputs,
        }
    }
}

#[derive(Debug, Default)]
pub struct QueueState {
    pub items: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct OutputState {
    pub active_id: String,
    pub outputs: Vec<crate::models::OutputInfo>,
}
