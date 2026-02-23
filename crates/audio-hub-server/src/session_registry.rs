//! In-memory playback session registry.
//!
//! Tracks session identity/lease metadata. Playback state migration to
//! per-session transport is handled separately.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::models::SessionMode;

const DEFAULT_LEASE_TTL_SEC: u64 = 30;

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub mode: SessionMode,
    pub client_id: String,
    pub app_version: String,
    pub owner: Option<String>,
    pub active_output_id: Option<String>,
    pub queue_len: usize,
    pub created_at: Instant,
    pub last_seen: Instant,
    pub lease_ttl: Duration,
    pub heartbeat_state: Option<String>,
    pub battery: Option<f32>,
}

#[derive(Default)]
struct SessionStore {
    by_id: HashMap<String, SessionRecord>,
    by_key: HashMap<(String, String), String>,
}

fn store() -> &'static Mutex<SessionStore> {
    static STORE: OnceLock<Mutex<SessionStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(SessionStore::default()))
}

fn mode_key(mode: &SessionMode) -> &'static str {
    match mode {
        SessionMode::Remote => "remote",
        SessionMode::Local => "local",
    }
}

pub fn create_or_refresh(
    name: String,
    mode: SessionMode,
    client_id: String,
    app_version: String,
    owner: Option<String>,
    lease_ttl_sec: Option<u64>,
) -> (String, u64) {
    let ttl = lease_ttl_sec.unwrap_or(DEFAULT_LEASE_TTL_SEC).max(5);
    let ttl_dur = Duration::from_secs(ttl);
    let now = Instant::now();
    let key = (mode_key(&mode).to_string(), client_id.clone());

    let mut store = store().lock().unwrap_or_else(|err| err.into_inner());
    if let Some(existing_id) = store.by_key.get(&key).cloned() {
        if let Some(existing) = store.by_id.get_mut(&existing_id) {
            existing.name = name;
            existing.app_version = app_version;
            existing.owner = owner;
            existing.last_seen = now;
            existing.lease_ttl = ttl_dur;
        }
        return (existing_id, ttl);
    }

    let id = format!("sess:{}", Uuid::new_v4());
    store.by_key.insert(key, id.clone());
    store.by_id.insert(
        id.clone(),
        SessionRecord {
            id: id.clone(),
            name,
            mode,
            client_id,
            app_version,
            owner,
            active_output_id: None,
            queue_len: 0,
            created_at: now,
            last_seen: now,
            lease_ttl: ttl_dur,
            heartbeat_state: None,
            battery: None,
        },
    );
    (id, ttl)
}

pub fn heartbeat(session_id: &str, state: String, battery: Option<f32>) -> Result<(), ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    session.last_seen = Instant::now();
    session.heartbeat_state = Some(state);
    session.battery = battery;
    Ok(())
}

pub fn list_sessions() -> Vec<SessionRecord> {
    store()
        .lock()
        .map(|s| s.by_id.values().cloned().collect())
        .unwrap_or_default()
}
