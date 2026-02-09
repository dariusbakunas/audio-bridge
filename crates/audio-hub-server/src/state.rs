//! Shared application state.
//!
//! Holds the in-memory library, playback state, and provider registries.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::{Condvar};
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;

use crate::bridge::{BridgeCommand, BridgePlayer};
use crate::config::BridgeConfigResolved;
use crate::events::{EventBus, LogBus};
use crate::library::LibraryIndex;
use crate::output_controller::OutputController;
use crate::playback_manager::PlaybackManager;
use crate::metadata_db::MetadataDb;
use crate::musicbrainz::MusicBrainzClient;

#[derive(Clone)]
pub struct MetadataWake {
    inner: Arc<(Mutex<u64>, Condvar)>,
}

impl MetadataWake {
    pub fn new() -> Self {
        Self {
            inner: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    pub fn notify(&self) {
        let (lock, cvar) = &*self.inner;
        let mut seq = lock.lock().expect("metadata wake lock");
        *seq = seq.wrapping_add(1);
        cvar.notify_all();
    }

    pub fn wait(&self, last_seen: &mut u64) {
        let (lock, cvar) = &*self.inner;
        let mut seq = lock.lock().expect("metadata wake lock");
        while *seq == *last_seen {
            seq = cvar.wait(seq).expect("metadata wake wait");
        }
        *last_seen = *seq;
    }
}

/// Snapshot of current playback state used for API responses and UI.
#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    /// Currently playing path (absolute).
    pub now_playing: Option<PathBuf>,
    /// True when playback is paused.
    pub paused: bool,
    /// Tracks explicit user pause (distinct from auto-pauses).
    pub user_paused: bool,
    /// Duration in milliseconds (best-effort).
    pub duration_ms: Option<u64>,
    /// Elapsed time in milliseconds (best-effort).
    pub elapsed_ms: Option<u64>,
    /// Source sample rate.
    pub sample_rate: Option<u32>,
    /// Source channel count.
    pub channels: Option<u16>,
    /// Output device name (if known).
    pub output_device: Option<String>,
    /// Codec name (best-effort).
    pub source_codec: Option<String>,
    /// Source bit depth (best-effort).
    pub source_bit_depth: Option<u16>,
    /// Container/extension hint (best-effort).
    pub container: Option<String>,
    /// Output sample format (e.g. I16/I32/F32).
    pub output_sample_format: Option<String>,
    /// Whether resampling is active.
    pub resampling: Option<bool>,
    /// Source rate (Hz) when resampling.
    pub resample_from_hz: Option<u32>,
    /// Output rate (Hz) when resampling.
    pub resample_to_hz: Option<u32>,
    /// Output device buffer size (frames).
    pub buffer_size_frames: Option<u32>,
    /// Current buffered frames (best-effort).
    pub buffered_frames: Option<u64>,
    /// Queue capacity in frames (best-effort).
    pub buffer_capacity_frames: Option<u64>,
    /// True when history has a previous track available.
    pub has_previous: Option<bool>,
    /// Auto-advance is in flight (prevents double-advance).
    pub auto_advance_in_flight: bool,
    /// Seek is in flight (prevents false end-of-track).
    pub seek_in_flight: bool,
    /// Manual next is in flight (suppresses auto-advance).
    pub manual_advance_in_flight: bool,
}

/// Grouped metadata dependencies for handlers/services.
pub struct MetadataState {
    /// Metadata database.
    pub db: MetadataDb,
    /// Optional MusicBrainz client for enrichment.
    pub musicbrainz: Option<Arc<MusicBrainzClient>>,
    /// Wake signal for metadata background jobs.
    pub wake: MetadataWake,
}

/// Grouped playback dependencies.
pub struct PlaybackState {
    /// Playback manager (queue + transport).
    pub manager: PlaybackManager,
    /// Device selections (local + per-bridge).
    pub device_selection: DeviceSelectionState,
}

/// Grouped provider state.
pub struct ProviderState {
    /// Bridge provider state (active bridge, discovery, transport).
    pub bridge: Arc<BridgeProviderState>,
    /// Local provider state (optional local playback).
    pub local: Arc<LocalProviderState>,
}

/// Grouped output dependencies.
pub struct OutputState {
    /// Output controller facade.
    pub controller: OutputController,
}

/// Shared application state for Actix handlers and background workers.
pub struct AppState {
    /// Library index and root.
    pub library: RwLock<LibraryIndex>,
    /// Grouped metadata dependencies.
    pub metadata: MetadataState,
    /// Grouped provider state.
    pub providers: ProviderState,
    /// Grouped playback state.
    pub playback: PlaybackState,
    /// Grouped output dependencies.
    pub output: OutputState,
    /// Event bus for UI subscriptions.
    pub events: EventBus,
    /// Log stream for UI subscriptions.
    pub log_bus: Arc<LogBus>,
}

impl AppState {
    pub fn new(
        library: LibraryIndex,
        metadata_db: MetadataDb,
        musicbrainz: Option<Arc<MusicBrainzClient>>,
        metadata_wake: MetadataWake,
        bridge: Arc<BridgeProviderState>,
        local: Arc<LocalProviderState>,
        playback_manager: PlaybackManager,
        device_selection: DeviceSelectionState,
        events: EventBus,
        log_bus: Arc<LogBus>,
    ) -> Self {
        Self {
            library: RwLock::new(library),
            metadata: MetadataState {
                db: metadata_db,
                musicbrainz,
                wake: metadata_wake,
            },
            providers: ProviderState { bridge, local },
            playback: PlaybackState {
                manager: playback_manager,
                device_selection,
            },
            output: OutputState {
                controller: OutputController::default(),
            },
            events,
            log_bus,
        }
    }
}

/// Discovered bridge entry from mDNS.
#[derive(Clone, Debug)]
pub struct DiscoveredBridge {
    /// Bridge config with resolved fields.
    pub bridge: crate::config::BridgeConfigResolved,
    /// Last-seen timestamp used for expiry.
    pub last_seen: std::time::Instant,
}

/// Queue state backing the server queue service.
#[derive(Debug, Default)]
pub struct QueueState {
    /// Ordered list of queued paths.
    pub items: Vec<PathBuf>,
}

/// Bridge-specific runtime state.
#[derive(Debug)]
pub struct BridgeState {
    /// Known bridges from config + discovery.
    pub bridges: Vec<BridgeConfigResolved>,
    /// Active bridge id (if selected).
    pub active_bridge_id: Option<String>,
    /// Active output id (if selected).
    pub active_output_id: Option<String>,
}

/// Shared state for the bridge output provider.
pub struct BridgeProviderState {
    /// Command channel for the active bridge player.
    pub player: Arc<Mutex<BridgePlayer>>,
    /// Active bridge/output selection.
    pub bridges: Arc<Mutex<BridgeState>>,
    /// Online flag for active bridge.
    pub bridge_online: Arc<AtomicBool>,
    /// Discovered bridges keyed by id.
    pub discovered_bridges: Arc<Mutex<std::collections::HashMap<String, DiscoveredBridge>>>,
    /// Public base URL for stream endpoints.
    pub public_base_url: String,
}

impl BridgeProviderState {
    /// Construct bridge provider state from runtime pieces.
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

/// Shared state for local output provider.
pub struct LocalProviderState {
    /// Whether local outputs are enabled.
    pub enabled: bool,
    /// Provider id.
    pub id: String,
    /// Provider display name.
    pub name: String,
    /// Command channel for local playback.
    pub player: Arc<Mutex<BridgePlayer>>,
    /// Local playback running flag.
    pub running: Arc<AtomicBool>,
}

/// Selected output devices for local and bridge providers.
#[derive(Clone)]
pub struct DeviceSelectionState {
    /// Selected local device name (if any).
    pub local: Arc<Mutex<Option<String>>>,
    /// Selected device id by bridge id.
    pub bridge: Arc<Mutex<std::collections::HashMap<String, String>>>,
}
