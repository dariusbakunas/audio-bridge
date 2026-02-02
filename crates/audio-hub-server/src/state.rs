use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;

use crate::bridge::{BridgeCommand, BridgePlayer};
use crate::config::BridgeConfigResolved;
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
    pub output_device: Option<String>,
    pub auto_advance_in_flight: bool,
}

pub struct AppState {
    pub library: RwLock<LibraryIndex>,
    pub player: Arc<Mutex<BridgePlayer>>,
    pub status: Arc<Mutex<PlayerStatus>>,
    pub queue: Arc<Mutex<QueueState>>,
    pub bridges: Arc<Mutex<BridgeState>>,
    pub bridge_online: Arc<AtomicBool>,
    pub discovered_bridges: Arc<Mutex<std::collections::HashMap<String, DiscoveredBridge>>>,
}

impl AppState {
    pub fn new(
        library: LibraryIndex,
        cmd_tx: Sender<BridgeCommand>,
        status: Arc<Mutex<PlayerStatus>>,
        queue: Arc<Mutex<QueueState>>,
        bridges: Arc<Mutex<BridgeState>>,
        bridge_online: Arc<AtomicBool>,
        discovered_bridges: Arc<Mutex<std::collections::HashMap<String, DiscoveredBridge>>>,
    ) -> Self {
        Self {
            library: RwLock::new(library),
            player: Arc::new(Mutex::new(BridgePlayer { cmd_tx })),
            status,
            queue,
            bridges,
            bridge_online,
            discovered_bridges,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveredBridge {
    pub bridge: crate::config::BridgeConfigResolved,
    pub last_seen: std::time::Instant,
}

#[derive(Debug, Default)]
pub struct QueueState {
    pub items: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct BridgeState {
    pub bridges: Vec<BridgeConfigResolved>,
    pub active_bridge_id: String,
    pub active_output_id: String,
}
