//! Shared application state.
//!
//! Holds the in-memory library, playback state, and provider registries.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;

use crate::bridge::{BridgeCommand, BridgePlayer};
use crate::config::BridgeConfigResolved;
use crate::library::LibraryIndex;
use crate::output_controller::OutputController;
use crate::playback_manager::PlaybackManager;

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
    pub source_codec: Option<String>,
    pub source_bit_depth: Option<u16>,
    pub container: Option<String>,
    pub output_sample_format: Option<String>,
    pub resampling: Option<bool>,
    pub resample_from_hz: Option<u32>,
    pub resample_to_hz: Option<u32>,
    pub buffer_size_frames: Option<u32>,
    pub buffered_frames: Option<u64>,
    pub buffer_capacity_frames: Option<u64>,
    pub auto_advance_in_flight: bool,
    pub seek_in_flight: bool,
}

pub struct AppState {
    pub library: RwLock<LibraryIndex>,
    pub bridge: Arc<BridgeProviderState>,
    pub local: Arc<LocalProviderState>,
    pub playback_manager: PlaybackManager,
    pub device_selection: DeviceSelectionState,
    pub output_controller: OutputController,
}

impl AppState {
    pub fn new(
        library: LibraryIndex,
        bridge: Arc<BridgeProviderState>,
        local: Arc<LocalProviderState>,
        playback_manager: PlaybackManager,
        device_selection: DeviceSelectionState,
    ) -> Self {
        Self {
            library: RwLock::new(library),
            bridge,
            local,
            playback_manager,
            device_selection,
            output_controller: OutputController::default(),
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
    pub active_bridge_id: Option<String>,
    pub active_output_id: Option<String>,
}

pub struct BridgeProviderState {
    pub player: Arc<Mutex<BridgePlayer>>,
    pub bridges: Arc<Mutex<BridgeState>>,
    pub bridge_online: Arc<AtomicBool>,
    pub discovered_bridges: Arc<Mutex<std::collections::HashMap<String, DiscoveredBridge>>>,
    pub public_base_url: String,
}

impl BridgeProviderState {
    pub fn new(
        cmd_tx: Sender<BridgeCommand>,
        bridges: Arc<Mutex<BridgeState>>,
        bridge_online: Arc<AtomicBool>,
        discovered_bridges: Arc<Mutex<std::collections::HashMap<String, DiscoveredBridge>>>,
        public_base_url: String,
    ) -> Self {
        Self {
            player: Arc::new(Mutex::new(BridgePlayer { cmd_tx })),
            bridges,
            bridge_online,
            discovered_bridges,
            public_base_url,
        }
    }
}


pub struct LocalProviderState {
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub player: Arc<Mutex<BridgePlayer>>,
    pub running: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct DeviceSelectionState {
    pub local: Arc<Mutex<Option<String>>>,
    pub bridge: Arc<Mutex<std::collections::HashMap<String, String>>>,
}
