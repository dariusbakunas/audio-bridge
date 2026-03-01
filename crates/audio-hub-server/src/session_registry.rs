//! In-memory playback session registry.
//!
//! Tracks session identity/lease metadata. Playback state migration to
//! per-session transport is handled separately.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::models::SessionMode;

const DEFAULT_LEASE_TTL_SEC: u64 = 30;

/// In-memory representation of a playback session.
///
/// A session tracks ownership (output lock), queue/history state, and lease info.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    /// Stable session id (`sess:<uuid>`).
    pub id: String,
    /// User-visible session name.
    pub name: String,
    /// Session mode (`remote` or `local`).
    pub mode: SessionMode,
    /// Client identity that owns this session.
    pub client_id: String,
    /// Client app version.
    pub app_version: String,
    /// Optional owner tag (for example `ios-app`, `web-ui`).
    pub owner: Option<String>,
    /// Currently selected output id, if any.
    pub active_output_id: Option<String>,
    /// Number of queued upcoming tracks.
    pub queue_len: usize,
    /// Currently playing track id, if any.
    pub now_playing: Option<i64>,
    /// Upcoming queue items.
    pub queue_items: Vec<i64>,
    /// Recently played items (oldest at front, newest at back).
    pub history: VecDeque<i64>,
    /// Creation timestamp.
    pub created_at: Instant,
    /// Last heartbeat/activity timestamp.
    pub last_seen: Instant,
    /// Session lease TTL. `0` means never expires.
    pub lease_ttl: Duration,
    /// Last heartbeat state string (foreground/background etc).
    pub heartbeat_state: Option<String>,
    /// Optional battery value reported by client.
    pub battery: Option<f32>,
}

#[derive(Default)]
struct SessionStore {
    by_id: HashMap<String, SessionRecord>,
    by_key: HashMap<(String, String), String>,
    output_locks: HashMap<String, String>,
    bridge_locks: HashMap<String, String>,
}

/// Return global in-memory session registry store.
fn store() -> &'static Mutex<SessionStore> {
    static STORE: OnceLock<Mutex<SessionStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(SessionStore::default()))
}

/// Normalize session mode into registry key segment.
fn mode_key(mode: &SessionMode) -> &'static str {
    match mode {
        SessionMode::Remote => "remote",
        SessionMode::Local => "local",
    }
}

/// Normalize session display name for identity comparisons.
fn session_name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Build stable identity key for create-or-refresh semantics.
fn session_identity_key(mode: &SessionMode, name: &str, client_id: &str) -> (String, String) {
    let identity = match mode {
        SessionMode::Remote => session_name_key(name),
        SessionMode::Local => client_id.trim().to_string(),
    };
    (mode_key(mode).to_string(), identity)
}

/// Create a new session or refresh an existing one with the same identity key.
///
/// Identity is `(mode, normalized_name)` for remote sessions and `(mode, client_id)`
/// for local sessions. Returns `(session_id, lease_ttl_sec_effective)`.
pub fn create_or_refresh(
    name: String,
    mode: SessionMode,
    client_id: String,
    app_version: String,
    owner: Option<String>,
    lease_ttl_sec: Option<u64>,
) -> (String, u64) {
    let never_expires = lease_ttl_sec == Some(0);
    let ttl = if never_expires {
        0
    } else {
        lease_ttl_sec.unwrap_or(DEFAULT_LEASE_TTL_SEC).max(5)
    };
    let ttl_dur = Duration::from_secs(ttl);
    let now = Instant::now();
    let key = session_identity_key(&mode, &name, &client_id);

    let mut store = store().lock().unwrap_or_else(|err| err.into_inner());
    if let Some(existing_id) = store.by_key.get(&key).cloned() {
        if let Some(existing) = store.by_id.get_mut(&existing_id) {
            existing.name = name;
            if matches!(mode, SessionMode::Local) {
                existing.client_id = client_id;
            }
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
            now_playing: None,
            queue_items: Vec::new(),
            history: VecDeque::new(),
            created_at: now,
            last_seen: now,
            lease_ttl: ttl_dur,
            heartbeat_state: None,
            battery: None,
        },
    );
    (id, ttl)
}

/// Update session heartbeat metadata and refresh `last_seen`.
pub fn heartbeat(session_id: &str, state: String, battery: Option<f32>) -> Result<(), ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    session.last_seen = Instant::now();
    session.heartbeat_state = Some(state);
    session.battery = battery;
    Ok(())
}

/// List all sessions without visibility filtering.
pub fn list_sessions() -> Vec<SessionRecord> {
    store()
        .lock()
        .map(|s| s.by_id.values().cloned().collect())
        .unwrap_or_default()
}

/// List sessions visible to a specific client.
///
/// Remote sessions are visible to all clients. Local sessions are only visible to
/// the owner `client_id`.
pub fn list_sessions_visible(viewer_client_id: Option<&str>) -> Vec<SessionRecord> {
    list_sessions()
        .into_iter()
        .filter(|session| {
            if !matches!(session.mode, SessionMode::Local) {
                return true;
            }
            match viewer_client_id {
                Some(client_id) => client_id == session.client_id,
                None => false,
            }
        })
        .collect()
}

/// Return current output and bridge lock snapshots as sorted `(key, session_id)` pairs.
pub fn lock_snapshot() -> (Vec<(String, String)>, Vec<(String, String)>) {
    let store = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let mut output_locks: Vec<(String, String)> = store
        .output_locks
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    output_locks.sort_by(|a, b| a.0.cmp(&b.0));
    let mut bridge_locks: Vec<(String, String)> = store
        .bridge_locks
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    bridge_locks.sort_by(|a, b| a.0.cmp(&b.0));
    (output_locks, bridge_locks)
}

/// Fetch a session by id.
pub fn get_session(session_id: &str) -> Option<SessionRecord> {
    store()
        .lock()
        .ok()
        .and_then(|s| s.by_id.get(session_id).cloned())
}

/// Fetch a session by id, applying local-session visibility checks.
pub fn get_session_visible(
    session_id: &str,
    viewer_client_id: Option<&str>,
) -> Option<SessionRecord> {
    let session = get_session(session_id)?;
    if matches!(session.mode, SessionMode::Local) {
        let Some(client_id) = viewer_client_id else {
            return None;
        };
        if session.client_id != client_id {
            return None;
        }
    }
    Some(session)
}

/// Touch a session lease timestamp.
///
/// Returns `true` when the session exists.
pub fn touch_session(session_id: &str) -> bool {
    let mut store = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    let Some(session) = store.by_id.get_mut(session_id) else {
        return false;
    };
    session.last_seen = Instant::now();
    true
}

/// Snapshot of queue state for a session.
#[derive(Clone, Debug)]
pub struct SessionQueueSnapshot {
    /// Current playing item.
    pub now_playing: Option<i64>,
    /// Upcoming items.
    pub queue_items: Vec<i64>,
    /// Recently played history.
    pub history: VecDeque<i64>,
}

/// Get queue snapshot for a session.
pub fn queue_snapshot(session_id: &str) -> Result<SessionQueueSnapshot, ()> {
    let store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get(session_id).ok_or(())?;
    Ok(SessionQueueSnapshot {
        now_playing: session.now_playing.clone(),
        queue_items: session.queue_items.clone(),
        history: session.history.clone(),
    })
}

/// Append unique track ids to the end of queue.
///
/// Returns number of inserted items.
pub fn queue_add_track_ids(session_id: &str, track_ids: Vec<i64>) -> Result<usize, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let mut added = 0usize;
    for track_id in track_ids {
        if session.queue_items.iter().any(|id| id == &track_id) {
            continue;
        }
        session.queue_items.push(track_id);
        added += 1;
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(added)
}

/// Insert unique track ids at the front of the upcoming queue in given order.
///
/// Returns number of inserted items.
pub fn queue_add_next_track_ids(session_id: &str, track_ids: Vec<i64>) -> Result<usize, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let mut added = 0usize;
    for track_id in track_ids {
        if session.queue_items.iter().any(|id| id == &track_id) {
            continue;
        }
        session.queue_items.insert(added, track_id);
        added += 1;
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(added)
}

/// Remove one upcoming queue entry by track id.
///
/// Returns `true` when an entry was removed.
pub fn queue_remove_track_id(session_id: &str, track_id: i64) -> Result<bool, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let removed = if let Some(pos) = session.queue_items.iter().position(|id| *id == track_id) {
        session.queue_items.remove(pos);
        true
    } else {
        false
    };
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(removed)
}

/// Clear upcoming queue and/or history for a session.
pub fn queue_clear(session_id: &str, clear_queue: bool, clear_history: bool) -> Result<(), ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    if clear_queue {
        session.queue_items.clear();
    }
    if clear_history {
        session.history.clear();
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(())
}

/// Jump playback context to a specific track id.
///
/// Search order:
/// 1. Upcoming queue (`queue_items`)
/// 2. Current `now_playing`
/// 3. Backwards through `history`
///
/// Returns `Ok(true)` when the target track exists and state was updated.
pub fn queue_play_from(session_id: &str, track_id: i64) -> Result<bool, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let found = if let Some(pos) = session.queue_items.iter().position(|id| *id == track_id) {
        if let Some(current) = session.now_playing.take() {
            if session
                .history
                .back()
                .map(|last| last != &current)
                .unwrap_or(true)
            {
                session.history.push_back(current);
            }
        }
        let selected = session.queue_items[pos];
        session.queue_items.drain(0..=pos);
        session.now_playing = Some(selected);
        true
    } else if session.now_playing == Some(track_id) {
        true
    } else {
        let mut matched_history = false;
        while let Some(prev) = session.history.pop_back() {
            if let Some(current) = session.now_playing.take() {
                if current != prev {
                    session.queue_items.insert(0, current);
                }
            }
            session.now_playing = Some(prev);
            if prev == track_id {
                matched_history = true;
                break;
            }
        }
        matched_history
    };
    if !found {
        return Ok(false);
    }
    if session.history.len() > 100 {
        let _ = session.history.pop_front();
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(true)
}

/// Advance to next track from the upcoming queue.
///
/// Moves previous `now_playing` into history. Returns selected track id.
pub fn queue_next_track_id(session_id: &str) -> Result<Option<i64>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let next = if session.queue_items.is_empty() {
        None
    } else {
        let track_id = session.queue_items.remove(0);
        if let Some(current) = session.now_playing.take() {
            if session
                .history
                .back()
                .map(|last| last != &current)
                .unwrap_or(true)
            {
                session.history.push_back(current);
            }
        }
        session.now_playing = Some(track_id);
        if session.history.len() > 100 {
            let _ = session.history.pop_front();
        }
        Some(track_id)
    };
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(next)
}

/// Go back to previous track from history.
///
/// Reinserts current `now_playing` to queue front when appropriate.
pub fn queue_previous_track_id(session_id: &str) -> Result<Option<i64>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    while let Some(prev) = session.history.pop_back() {
        if let Some(current) = session.now_playing.take() {
            if current != prev {
                session.queue_items.insert(0, current);
            }
        }
        session.now_playing = Some(prev);
        session.queue_len = session.queue_items.len();
        session.last_seen = Instant::now();
        return Ok(Some(prev));
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(None)
}

/// Errors validating a session's selected output lock.
#[derive(Clone, Debug)]
pub enum BoundOutputError {
    /// Session id does not exist.
    SessionNotFound,
    /// Session exists but no output is selected.
    NoOutputSelected,
    /// Session selected an output but lock map is inconsistent.
    OutputLockMissing { output_id: String },
    /// Selected output is owned by a different session.
    OutputInUse {
        output_id: String,
        held_by_session_id: String,
    },
}

/// Require a session to have a selected and currently owned output.
///
/// Returns that output id when lock ownership is valid.
pub fn require_bound_output(session_id: &str) -> Result<String, BoundOutputError> {
    let mut store = store()
        .lock()
        .map_err(|_| BoundOutputError::SessionNotFound)?;
    let output_id = store
        .by_id
        .get(session_id)
        .ok_or(BoundOutputError::SessionNotFound)?
        .active_output_id
        .clone()
        .ok_or(BoundOutputError::NoOutputSelected)?;
    match store.output_locks.get(&output_id) {
        Some(holder) if holder == session_id => {
            if let Some(session) = store.by_id.get_mut(session_id) {
                session.last_seen = Instant::now();
            }
            Ok(output_id)
        }
        Some(holder) => Err(BoundOutputError::OutputInUse {
            output_id,
            held_by_session_id: holder.clone(),
        }),
        None => Err(BoundOutputError::OutputLockMissing { output_id }),
    }
}

/// Return current owner session id for an output lock.
pub fn output_lock_owner(output_id: &str) -> Option<String> {
    store()
        .lock()
        .ok()
        .and_then(|s| s.output_locks.get(output_id).cloned())
}

/// Errors produced by `bind_output`.
#[derive(Clone, Debug)]
pub enum BindError {
    /// Session id does not exist.
    SessionNotFound,
    /// Target output is already owned by another session.
    OutputInUse {
        output_id: String,
        held_by_session_id: String,
    },
    /// Target bridge family is already owned by another session.
    BridgeInUse {
        bridge_id: String,
        held_by_session_id: String,
    },
}

/// Bind an output to a session, optionally forcing takeover.
///
/// For bridge outputs, this also acquires a bridge-family lock to prevent concurrent
/// selection of sibling outputs by different sessions.
pub fn bind_output(session_id: &str, output_id: &str, force: bool) -> Result<(), BindError> {
    let mut store = store().lock().map_err(|_| BindError::SessionNotFound)?;
    if !store.by_id.contains_key(session_id) {
        return Err(BindError::SessionNotFound);
    }

    let mut displaced_session_id = None;
    let requested_bridge_id = parse_bridge_id(output_id);
    if let Some(bridge_id) = requested_bridge_id.as_deref() {
        if let Some(holder) = store.bridge_locks.get(bridge_id).cloned() {
            if holder != session_id {
                if !force {
                    return Err(BindError::BridgeInUse {
                        bridge_id: bridge_id.to_string(),
                        held_by_session_id: holder,
                    });
                }
                displaced_session_id = Some(holder);
            }
        }
    }

    if let Some(holder) = store.output_locks.get(output_id).cloned() {
        if holder != session_id {
            if !force {
                return Err(BindError::OutputInUse {
                    output_id: output_id.to_string(),
                    held_by_session_id: holder,
                });
            }
            if displaced_session_id.is_none() {
                displaced_session_id = Some(holder);
            }
        }
    }

    let previous_output = store
        .by_id
        .get(session_id)
        .and_then(|s| s.active_output_id.clone());

    if let Some(prev) = previous_output.as_deref() {
        if prev != output_id {
            store.output_locks.remove(prev);
            if let Some(prev_bridge_id) = parse_bridge_id(prev) {
                if store
                    .bridge_locks
                    .get(&prev_bridge_id)
                    .map(|id| id.as_str())
                    == Some(session_id)
                {
                    store.bridge_locks.remove(&prev_bridge_id);
                }
            }
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
    if let Some(bridge_id) = requested_bridge_id {
        store.bridge_locks.insert(bridge_id, session_id.to_string());
    }
    if let Some(session) = store.by_id.get_mut(session_id) {
        session.active_output_id = Some(output_id.to_string());
        session.last_seen = Instant::now();
    }

    Ok(())
}

/// Release currently bound output for a session.
///
/// Returns released output id when one was set.
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
        if let Some(bridge_id) = parse_bridge_id(output_id) {
            if store.bridge_locks.get(&bridge_id).map(|id| id.as_str()) == Some(session_id) {
                store.bridge_locks.remove(&bridge_id);
            }
        }
    }
    Ok(released)
}

/// Delete a session and release its locks.
///
/// Returns the previously bound output id, if any.
pub fn delete_session(session_id: &str) -> Result<Option<String>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let Some(removed) = store.by_id.remove(session_id) else {
        return Err(());
    };
    let key = session_identity_key(&removed.mode, &removed.name, &removed.client_id);
    if store.by_key.get(&key).map(|id| id.as_str()) == Some(session_id) {
        store.by_key.remove(&key);
    }
    if let Some(output_id) = removed.active_output_id.as_deref() {
        if store.output_locks.get(output_id).map(|id| id.as_str()) == Some(session_id) {
            store.output_locks.remove(output_id);
        }
        if let Some(bridge_id) = parse_bridge_id(output_id) {
            if store.bridge_locks.get(&bridge_id).map(|id| id.as_str()) == Some(session_id) {
                store.bridge_locks.remove(&bridge_id);
            }
        }
    }
    Ok(removed.active_output_id)
}

#[allow(dead_code)]
/// Purge expired sessions based on lease TTL and return removed session ids.
pub fn purge_expired() -> Vec<String> {
    let now = Instant::now();
    let mut store = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return Vec::new(),
    };

    let expired_ids: Vec<String> = store
        .by_id
        .iter()
        .filter_map(|(id, session)| {
            if session.lease_ttl.as_secs() == 0 {
                return None;
            }
            let age = now.saturating_duration_since(session.last_seen);
            if age >= session.lease_ttl {
                Some(id.clone())
            } else {
                None
            }
        })
        .collect();

    if expired_ids.is_empty() {
        return Vec::new();
    }

    for session_id in &expired_ids {
        let Some(removed) = store.by_id.remove(session_id) else {
            continue;
        };
        let key = session_identity_key(&removed.mode, &removed.name, &removed.client_id);
        if store.by_key.get(&key).map(|id| id.as_str()) == Some(session_id.as_str()) {
            store.by_key.remove(&key);
        }
        if let Some(output_id) = removed.active_output_id.as_deref() {
            if store.output_locks.get(output_id).map(|id| id.as_str()) == Some(session_id.as_str())
            {
                store.output_locks.remove(output_id);
            }
            if let Some(bridge_id) = parse_bridge_id(output_id) {
                if store.bridge_locks.get(&bridge_id).map(|id| id.as_str())
                    == Some(session_id.as_str())
                {
                    store.bridge_locks.remove(&bridge_id);
                }
            }
        }
    }

    expired_ids
}

/// Parse bridge id from output id format `bridge:<bridge_id>:<device_id>`.
fn parse_bridge_id(output_id: &str) -> Option<String> {
    let mut parts = output_id.splitn(3, ':');
    let kind = parts.next().unwrap_or("");
    let bridge_id = parts.next().unwrap_or("");
    let device_id = parts.next().unwrap_or("");
    if kind == "bridge" && !bridge_id.is_empty() && !device_id.is_empty() {
        Some(bridge_id.to_string())
    } else {
        None
    }
}

#[cfg(test)]
pub fn reset_for_tests() {
    if let Ok(mut store) = store().lock() {
        *store = SessionStore::default();
    }
}

#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("test lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        super::test_lock()
    }

    fn make_session(name: &str, client_id: &str) -> String {
        create_or_refresh(
            name.to_string(),
            SessionMode::Remote,
            client_id.to_string(),
            "test".to_string(),
            None,
            None,
        )
        .0
    }

    #[test]
    fn create_or_refresh_supports_never_expiring_sessions() {
        let _guard = test_guard();
        reset_for_tests();
        let (sid, ttl) = create_or_refresh(
            "Default".to_string(),
            SessionMode::Remote,
            "client-default".to_string(),
            "test".to_string(),
            None,
            Some(0),
        );
        assert_eq!(ttl, 0);
        let session = get_session(&sid).expect("session");
        assert_eq!(session.lease_ttl.as_secs(), 0);
    }

    #[test]
    fn create_or_refresh_reuses_session_by_name_across_clients() {
        let _guard = test_guard();
        reset_for_tests();
        let (a, _) = create_or_refresh(
            "Living Room".to_string(),
            SessionMode::Remote,
            "client-a".to_string(),
            "test".to_string(),
            None,
            None,
        );
        let (b, _) = create_or_refresh(
            "Living Room".to_string(),
            SessionMode::Remote,
            "client-b".to_string(),
            "test".to_string(),
            None,
            None,
        );
        assert_eq!(a, b);
        let session = get_session(&a).expect("session");
        assert_eq!(session.client_id, "client-a");
    }

    #[test]
    fn create_or_refresh_local_uses_client_id_identity() {
        let _guard = test_guard();
        reset_for_tests();
        let (a, _) = create_or_refresh(
            "Local".to_string(),
            SessionMode::Local,
            "client-a".to_string(),
            "test".to_string(),
            None,
            None,
        );
        let (b, _) = create_or_refresh(
            "Local".to_string(),
            SessionMode::Local,
            "client-b".to_string(),
            "test".to_string(),
            None,
            None,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn purge_expired_removes_expired_and_keeps_never_expiring() {
        let _guard = test_guard();
        reset_for_tests();
        let (expired_id, _) = create_or_refresh(
            "Temp".to_string(),
            SessionMode::Remote,
            "client-temp".to_string(),
            "test".to_string(),
            None,
            Some(5),
        );
        let (default_id, _) = create_or_refresh(
            "Default".to_string(),
            SessionMode::Remote,
            "client-default".to_string(),
            "test".to_string(),
            None,
            Some(0),
        );
        bind_output(&expired_id, "bridge:living:dev1", false).expect("bind expired");
        {
            let mut s = store().lock().expect("store");
            let record = s.by_id.get_mut(&expired_id).expect("expired session");
            record.last_seen = Instant::now() - Duration::from_secs(10);
        }

        let removed = purge_expired();
        assert_eq!(removed, vec![expired_id.clone()]);
        assert!(get_session(&expired_id).is_none());
        assert!(get_session(&default_id).is_some());
        let (output_locks, bridge_locks) = lock_snapshot();
        assert!(output_locks.is_empty());
        assert!(bridge_locks.is_empty());
    }

    #[test]
    fn bind_output_rejects_bridge_in_use() {
        let _guard = test_guard();
        reset_for_tests();
        let a = make_session("A", "a");
        let b = make_session("B", "b");

        bind_output(&a, "bridge:living:dev1", false).expect("bind a");
        let err = bind_output(&b, "bridge:living:dev2", false).expect_err("bind b should fail");
        match err {
            BindError::BridgeInUse {
                bridge_id,
                held_by_session_id,
            } => {
                assert_eq!(bridge_id, "living");
                assert_eq!(held_by_session_id, a);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn bind_output_rejects_output_in_use_non_bridge() {
        let _guard = test_guard();
        reset_for_tests();
        let a = make_session("A", "a");
        let b = make_session("B", "b");

        bind_output(&a, "local:host:device-1", false).expect("bind a");
        let err = bind_output(&b, "local:host:device-1", false).expect_err("bind b should fail");
        match err {
            BindError::OutputInUse {
                output_id,
                held_by_session_id,
            } => {
                assert_eq!(output_id, "local:host:device-1");
                assert_eq!(held_by_session_id, a);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn release_output_clears_output_and_bridge_locks() {
        let _guard = test_guard();
        reset_for_tests();
        let a = make_session("A", "a");
        bind_output(&a, "bridge:living:dev1", false).expect("bind");

        let (output_locks, bridge_locks) = lock_snapshot();
        assert_eq!(output_locks.len(), 1);
        assert_eq!(bridge_locks.len(), 1);

        let released = release_output(&a).expect("release");
        assert_eq!(released.as_deref(), Some("bridge:living:dev1"));

        let (output_locks, bridge_locks) = lock_snapshot();
        assert!(output_locks.is_empty());
        assert!(bridge_locks.is_empty());
    }

    #[test]
    fn delete_session_clears_output_and_bridge_locks() {
        let _guard = test_guard();
        reset_for_tests();
        let a = make_session("A", "a");
        bind_output(&a, "bridge:living:dev1", false).expect("bind");

        let released = delete_session(&a).expect("delete");
        assert_eq!(released.as_deref(), Some("bridge:living:dev1"));

        let (output_locks, bridge_locks) = lock_snapshot();
        assert!(output_locks.is_empty());
        assert!(bridge_locks.is_empty());
    }

    #[test]
    fn queue_play_from_selects_pending_queue_item() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q");
        let a = 101;
        let b = 102;
        let c = 103;
        queue_add_track_ids(&sid, vec![a, b, c]).expect("add tracks");
        let current = queue_next_track_id(&sid).expect("next").expect("current");
        assert_eq!(current, a);

        let found = queue_play_from(&sid, c).expect("play from");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing, Some(c));
        assert!(snapshot.queue_items.is_empty());
        assert_eq!(snapshot.history.back().copied(), Some(a));
    }

    #[test]
    fn queue_play_from_accepts_current_track() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q2");
        let a = 101;
        queue_add_track_ids(&sid, vec![a]).expect("add track");
        let _ = queue_next_track_id(&sid).expect("next");

        let found = queue_play_from(&sid, a).expect("play from current");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing, Some(a));
        assert!(snapshot.queue_items.is_empty());
    }

    #[test]
    fn queue_play_from_rewinds_history_to_target() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q3");
        let a = 101;
        let b = 102;
        let c = 103;
        queue_add_track_ids(&sid, vec![a, b, c]).expect("add");
        let _ = queue_next_track_id(&sid).expect("next a");
        let _ = queue_next_track_id(&sid).expect("next b");
        let _ = queue_next_track_id(&sid).expect("next c");

        let found = queue_play_from(&sid, a).expect("play from history");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing, Some(a));
        assert_eq!(snapshot.queue_items.len(), 2);
        assert_eq!(snapshot.queue_items[0], b);
        assert_eq!(snapshot.queue_items[1], c);
    }

    #[test]
    fn queue_play_from_returns_false_when_not_present_anywhere() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q4");
        let a = 101;
        queue_add_track_ids(&sid, vec![a]).expect("add");
        let _ = queue_next_track_id(&sid).expect("next");
        let missing = 999;

        let found = queue_play_from(&sid, missing).expect("play from missing");
        assert!(!found);
    }
}
