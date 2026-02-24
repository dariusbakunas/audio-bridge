//! In-memory playback session registry.
//!
//! Tracks session identity/lease metadata. Playback state migration to
//! per-session transport is handled separately.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
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
    pub now_playing: Option<PathBuf>,
    pub queue_items: Vec<PathBuf>,
    pub history: VecDeque<PathBuf>,
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
    bridge_locks: HashMap<String, String>,
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

pub fn get_session(session_id: &str) -> Option<SessionRecord> {
    store()
        .lock()
        .ok()
        .and_then(|s| s.by_id.get(session_id).cloned())
}

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

#[derive(Clone, Debug)]
pub struct SessionQueueSnapshot {
    pub now_playing: Option<PathBuf>,
    pub queue_items: Vec<PathBuf>,
    pub history: VecDeque<PathBuf>,
}

pub fn queue_snapshot(session_id: &str) -> Result<SessionQueueSnapshot, ()> {
    let store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get(session_id).ok_or(())?;
    Ok(SessionQueueSnapshot {
        now_playing: session.now_playing.clone(),
        queue_items: session.queue_items.clone(),
        history: session.history.clone(),
    })
}

pub fn queue_add_paths(session_id: &str, paths: Vec<PathBuf>) -> Result<usize, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let mut added = 0usize;
    for path in paths {
        if session.queue_items.iter().any(|p| p == &path) {
            continue;
        }
        session.queue_items.push(path);
        added += 1;
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(added)
}

pub fn queue_add_next_paths(session_id: &str, paths: Vec<PathBuf>) -> Result<usize, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let mut added = 0usize;
    for path in paths {
        if session.queue_items.iter().any(|p| p == &path) {
            continue;
        }
        session.queue_items.insert(added, path);
        added += 1;
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(added)
}

pub fn queue_remove_path(session_id: &str, path: &Path) -> Result<bool, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let removed = if let Some(pos) = session.queue_items.iter().position(|p| p == path) {
        session.queue_items.remove(pos);
        true
    } else {
        false
    };
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(removed)
}

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

pub fn queue_play_from(session_id: &str, path: &Path) -> Result<bool, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let found = if let Some(pos) = session.queue_items.iter().position(|p| p == path) {
        if let Some(current) = session.now_playing.take() {
            if session.history.back().map(|last| last != &current).unwrap_or(true) {
                session.history.push_back(current);
            }
        }
        let selected = session.queue_items[pos].clone();
        session.queue_items.drain(0..=pos);
        session.now_playing = Some(selected);
        true
    } else if session.now_playing.as_deref() == Some(path) {
        true
    } else {
        let mut matched_history = false;
        while let Some(prev) = session.history.pop_back() {
            if let Some(current) = session.now_playing.take() {
                if current != prev {
                    session.queue_items.insert(0, current);
                }
            }
            session.now_playing = Some(prev.clone());
            if prev.as_path() == path {
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

pub fn queue_next_path(session_id: &str) -> Result<Option<PathBuf>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    let next = if session.queue_items.is_empty() {
        None
    } else {
        let path = session.queue_items.remove(0);
        if let Some(current) = session.now_playing.take() {
            if session.history.back().map(|last| last != &current).unwrap_or(true) {
                session.history.push_back(current);
            }
        }
        session.now_playing = Some(path.clone());
        if session.history.len() > 100 {
            let _ = session.history.pop_front();
        }
        Some(path)
    };
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(next)
}

pub fn queue_previous_path(session_id: &str) -> Result<Option<PathBuf>, ()> {
    let mut store = store().lock().map_err(|_| ())?;
    let session = store.by_id.get_mut(session_id).ok_or(())?;
    while let Some(prev) = session.history.pop_back() {
        if let Some(current) = session.now_playing.take() {
            if current != prev {
                session.queue_items.insert(0, current);
            }
        }
        session.now_playing = Some(prev.clone());
        session.queue_len = session.queue_items.len();
        session.last_seen = Instant::now();
        return Ok(Some(prev));
    }
    session.queue_len = session.queue_items.len();
    session.last_seen = Instant::now();
    Ok(None)
}

#[derive(Clone, Debug)]
pub enum BoundOutputError {
    SessionNotFound,
    NoOutputSelected,
    OutputLockMissing { output_id: String },
    OutputInUse {
        output_id: String,
        held_by_session_id: String,
    },
}

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

pub fn output_lock_owner(output_id: &str) -> Option<String> {
    store()
        .lock()
        .ok()
        .and_then(|s| s.output_locks.get(output_id).cloned())
}

#[derive(Clone, Debug)]
pub enum BindError {
    SessionNotFound,
    OutputInUse { output_id: String, held_by_session_id: String },
    BridgeInUse { bridge_id: String, held_by_session_id: String },
}

pub fn bind_output(
    session_id: &str,
    output_id: &str,
    force: bool,
) -> Result<(), BindError> {
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

    let previous_output = store.by_id.get(session_id).and_then(|s| s.active_output_id.clone());

    if let Some(prev) = previous_output.as_deref() {
        if prev != output_id {
            store.output_locks.remove(prev);
            if let Some(prev_bridge_id) = parse_bridge_id(prev) {
                if store.bridge_locks.get(&prev_bridge_id).map(|id| id.as_str()) == Some(session_id) {
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
        if let Some(bridge_id) = parse_bridge_id(output_id) {
            if store.bridge_locks.get(&bridge_id).map(|id| id.as_str()) == Some(session_id) {
                store.bridge_locks.remove(&bridge_id);
            }
        }
    }
    Ok(removed.active_output_id)
}

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
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn test_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test lock poisoned")
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
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        let c = PathBuf::from("/music/c.flac");
        queue_add_paths(&sid, vec![a.clone(), b.clone(), c.clone()]).expect("add paths");
        let current = queue_next_path(&sid).expect("next").expect("current");
        assert_eq!(current, a);

        let found = queue_play_from(&sid, &c).expect("play from");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing.as_deref(), Some(c.as_path()));
        assert!(snapshot.queue_items.is_empty());
        assert_eq!(snapshot.history.back().map(|p| p.as_path()), Some(a.as_path()));
    }

    #[test]
    fn queue_play_from_accepts_current_track() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q2");
        let a = PathBuf::from("/music/a.flac");
        queue_add_paths(&sid, vec![a.clone()]).expect("add path");
        let _ = queue_next_path(&sid).expect("next");

        let found = queue_play_from(&sid, &a).expect("play from current");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing.as_deref(), Some(a.as_path()));
        assert!(snapshot.queue_items.is_empty());
    }

    #[test]
    fn queue_play_from_rewinds_history_to_target() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q3");
        let a = PathBuf::from("/music/a.flac");
        let b = PathBuf::from("/music/b.flac");
        let c = PathBuf::from("/music/c.flac");
        queue_add_paths(&sid, vec![a.clone(), b.clone(), c.clone()]).expect("add");
        let _ = queue_next_path(&sid).expect("next a");
        let _ = queue_next_path(&sid).expect("next b");
        let _ = queue_next_path(&sid).expect("next c");

        let found = queue_play_from(&sid, &a).expect("play from history");
        assert!(found);
        let snapshot = queue_snapshot(&sid).expect("snapshot");
        assert_eq!(snapshot.now_playing.as_deref(), Some(a.as_path()));
        assert_eq!(snapshot.queue_items.len(), 2);
        assert_eq!(snapshot.queue_items[0], b);
        assert_eq!(snapshot.queue_items[1], c);
    }

    #[test]
    fn queue_play_from_returns_false_when_not_present_anywhere() {
        let _guard = test_guard();
        reset_for_tests();
        let sid = make_session("Q", "q4");
        let a = PathBuf::from("/music/a.flac");
        queue_add_paths(&sid, vec![a.clone()]).expect("add");
        let _ = queue_next_path(&sid).expect("next");
        let missing = PathBuf::from("/music/missing.flac");

        let found = queue_play_from(&sid, &missing).expect("play from missing");
        assert!(!found);
    }
}
