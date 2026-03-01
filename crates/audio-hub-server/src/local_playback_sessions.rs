//! Local playback session registry.
//!
//! Tracks app sessions that resolve local stream URLs without participating
//! in output selection or hub playback state.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use uuid::Uuid;

/// Public snapshot of a registered local playback session.
#[derive(Clone, Debug)]
pub struct LocalPlaybackSession {
    /// Stable local session id (`local:<kind>:<uuid>`).
    pub session_id: String,
    /// Client kind (`ios`, `browser`, ...).
    pub kind: String,
    /// Human-readable client name.
    pub name: String,
    /// Client app version.
    pub app_version: String,
    /// Creation timestamp.
    pub created_at: Instant,
    /// Last-seen timestamp.
    pub last_seen: Instant,
}

#[derive(Default)]
struct LocalPlaybackStore {
    by_key: HashMap<(String, String), String>,
    by_session_id: HashMap<String, LocalPlaybackSessionInternal>,
}

#[derive(Clone, Debug)]
struct LocalPlaybackSessionInternal {
    session_id: String,
    kind: String,
    name: String,
    app_version: String,
    created_at: Instant,
    last_seen: Instant,
}

/// Return global in-memory local-session store.
fn store() -> &'static Mutex<LocalPlaybackStore> {
    static STORE: OnceLock<Mutex<LocalPlaybackStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(LocalPlaybackStore::default()))
}

/// Register or refresh a local playback session by `(kind, client_id)`.
pub fn register_session(
    kind: String,
    name: String,
    client_id: String,
    app_version: String,
) -> String {
    let mut store = store().lock().unwrap_or_else(|err| err.into_inner());
    let key = (kind.clone(), client_id.clone());
    if let Some(session_id) = store.by_key.get(&key).cloned() {
        if let Some(existing) = store.by_session_id.get_mut(&session_id) {
            existing.kind = kind;
            existing.name = name;
            existing.app_version = app_version;
            existing.last_seen = Instant::now();
        }
        return session_id;
    }

    let kind_segment = sanitize_segment(&kind);
    let session_id = format!("local:{kind_segment}:{}", Uuid::new_v4());
    let now = Instant::now();
    store.by_key.insert(key, session_id.clone());
    store.by_session_id.insert(
        session_id.clone(),
        LocalPlaybackSessionInternal {
            session_id: session_id.clone(),
            kind,
            name,
            app_version,
            created_at: now,
            last_seen: now,
        },
    );
    session_id
}

/// Returns `true` when a local playback session id is registered.
pub fn has_session(session_id: &str) -> bool {
    store()
        .lock()
        .map(|s| s.by_session_id.contains_key(session_id))
        .unwrap_or(false)
}

/// Refresh `last_seen` for a local playback session.
///
/// Returns `true` when the session exists.
pub fn touch_session(session_id: &str) -> bool {
    let mut store = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    let Some(session) = store.by_session_id.get_mut(session_id) else {
        return false;
    };
    session.last_seen = Instant::now();
    true
}

/// List all currently registered local playback sessions.
pub fn list_sessions() -> Vec<LocalPlaybackSession> {
    store()
        .lock()
        .map(|s| {
            s.by_session_id
                .values()
                .map(|entry| LocalPlaybackSession {
                    session_id: entry.session_id.clone(),
                    kind: entry.kind.clone(),
                    name: entry.name.clone(),
                    app_version: entry.app_version.clone(),
                    created_at: entry.created_at,
                    last_seen: entry.last_seen,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Sanitize free-form id segment for `local:<kind>:...` session ids.
fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
