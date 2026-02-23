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
    output_locks: HashMap<String, String>,
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

pub fn get_session(session_id: &str) -> Option<SessionRecord> {
    store()
        .lock()
        .ok()
        .and_then(|s| s.by_id.get(session_id).cloned())
}

#[derive(Clone, Debug)]
pub struct BindTransition {
    pub previous_output: Option<String>,
    pub displaced_session_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum BindError {
    SessionNotFound,
    OutputInUse { output_id: String, held_by_session_id: String },
}

pub fn bind_output(
    session_id: &str,
    output_id: &str,
    force: bool,
) -> Result<BindTransition, BindError> {
    let mut store = store().lock().map_err(|_| BindError::SessionNotFound)?;
    if !store.by_id.contains_key(session_id) {
        return Err(BindError::SessionNotFound);
    }

    let mut displaced_session_id = None;
    if let Some(holder) = store.output_locks.get(output_id).cloned() {
        if holder != session_id {
            if !force {
                return Err(BindError::OutputInUse {
                    output_id: output_id.to_string(),
                    held_by_session_id: holder,
                });
            }
            displaced_session_id = Some(holder);
        }
    }

    let previous_output = store
        .by_id
        .get(session_id)
        .and_then(|s| s.active_output_id.clone());

    if let Some(prev) = previous_output.as_deref() {
        if prev != output_id {
            store.output_locks.remove(prev);
        }
    }

    if let Some(displaced) = displaced_session_id.as_deref() {
        if let Some(displaced_session) = store.by_id.get_mut(displaced) {
            displaced_session.active_output_id = None;
            displaced_session.last_seen = Instant::now();
        }
    }

    store
        .output_locks
        .insert(output_id.to_string(), session_id.to_string());
    if let Some(session) = store.by_id.get_mut(session_id) {
        session.active_output_id = Some(output_id.to_string());
        session.last_seen = Instant::now();
    }

    Ok(BindTransition {
        previous_output,
        displaced_session_id,
    })
}

pub fn rollback_bind(
    session_id: &str,
    attempted_output_id: &str,
    transition: BindTransition,
) {
    let mut store = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    store.output_locks.remove(attempted_output_id);

    if let Some(session) = store.by_id.get_mut(session_id) {
        session.active_output_id = transition.previous_output.clone();
        session.last_seen = Instant::now();
    }

    if let Some(prev) = transition.previous_output {
        store.output_locks.insert(prev, session_id.to_string());
    }

    if let Some(displaced) = transition.displaced_session_id {
        if let Some(displaced_session) = store.by_id.get_mut(&displaced) {
            displaced_session.active_output_id = Some(attempted_output_id.to_string());
            displaced_session.last_seen = Instant::now();
        }
        store
            .output_locks
            .insert(attempted_output_id.to_string(), displaced);
    }
}

pub fn release_output(session_id: &str) -> Result<Option<String>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let released = {
        let session = store.by_id.get_mut(session_id).ok_or(())?;
        let released = session.active_output_id.take();
        session.last_seen = Instant::now();
        released
    };
    if let Some(output_id) = released.as_deref() {
        store.output_locks.remove(output_id);
    }
    Ok(released)
}

pub fn delete_session(session_id: &str) -> Result<Option<String>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let Some(removed) = store.by_id.remove(session_id) else {
        return Err(());
    };
    let key = (mode_key(&removed.mode).to_string(), removed.client_id.clone());
    if store.by_key.get(&key).map(|id| id.as_str()) == Some(session_id) {
        store.by_key.remove(&key);
    }
    if let Some(output_id) = removed.active_output_id.as_deref() {
        if store.output_locks.get(output_id).map(|id| id.as_str()) == Some(session_id) {
            store.output_locks.remove(output_id);
        }
    }
    Ok(removed.active_output_id)
}
