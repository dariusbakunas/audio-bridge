//! Browser playback session tracking and command dispatch.
//!
//! Manages connected browser receivers and provides helpers to send playback commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use actix::prelude::*;
use crossbeam_channel::Receiver;
use serde::Serialize;

use crate::bridge::BridgeCommand;
use crate::events::EventBus;
use crate::status_store::StatusStore;

/// Outbound messages to a browser websocket session.
#[derive(Message)]
#[rtype(result = "()")]
pub struct BrowserOutbound(pub String);

#[derive(Clone)]
pub struct BrowserProviderState {
    sessions: Arc<Mutex<HashMap<String, BrowserSession>>>,
    counter: Arc<AtomicUsize>,
}

#[derive(Clone)]
pub struct BrowserSession {
    pub id: String,
    pub name: String,
    pub sender: Recipient<BrowserOutbound>,
    pub last_duration_ms: Option<u64>,
}

impl BrowserProviderState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn register_session(&self, name: String, sender: Recipient<BrowserOutbound>) -> String {
        let id = format!("browser-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let session = BrowserSession {
            id: id.clone(),
            name,
            sender,
            last_duration_ms: None,
        };
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(id.clone(), session);
        }
        id
    }

    pub fn update_name(&self, id: &str, name: String) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(id) {
                session.name = name;
            }
        }
    }

    pub fn update_last_duration(&self, id: &str, duration_ms: Option<u64>) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get_mut(id) {
                session.last_duration_ms = duration_ms;
            }
        }
    }

    pub fn remove_session(&self, id: &str) -> Option<BrowserSession> {
        self.sessions.lock().ok().and_then(|mut s| s.remove(id))
    }

    pub fn list_sessions(&self) -> Vec<BrowserSession> {
        self.sessions
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn get_session(&self, id: &str) -> Option<BrowserSession> {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(id).cloned())
    }

    pub fn get_last_duration(&self, id: &str) -> Option<u64> {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(id).and_then(|session| session.last_duration_ms))
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrowserServerMessage {
    Hello { session_id: String },
    Play {
        url: String,
        path: String,
        start_paused: bool,
        seek_ms: Option<u64>,
    },
    PauseToggle,
    Stop,
    Seek { ms: u64 },
}

/// Spawn a worker that bridges playback commands to a browser session.
pub fn spawn_browser_worker(
    session_id: String,
    sender: Recipient<BrowserOutbound>,
    cmd_rx: Receiver<BridgeCommand>,
    status: StatusStore,
    queue: Arc<Mutex<crate::state::QueueState>>,
    events: EventBus,
    public_base_url: String,
) {
    std::thread::spawn(move || {
    let _ = (queue, events);
    for cmd in cmd_rx.iter() {
        match cmd {
                BridgeCommand::Quit => break,
                BridgeCommand::PauseToggle => {
                    let _ = send_json(&sender, BrowserServerMessage::PauseToggle);
                    status.on_pause_toggle();
                }
                BridgeCommand::Stop => {
                    let _ = send_json(&sender, BrowserServerMessage::Stop);
                    status.on_stop();
                }
                BridgeCommand::Seek { ms } => {
                    let _ = send_json(&sender, BrowserServerMessage::Seek { ms });
                    status.mark_seek_in_flight();
                }
                BridgeCommand::Play { path, seek_ms, start_paused, .. } => {
                    let url = build_stream_url_for(&path, &public_base_url);
                    let _ = send_json(
                        &sender,
                        BrowserServerMessage::Play {
                            url,
                            path: path.to_string_lossy().to_string(),
                            start_paused,
                            seek_ms,
                        },
                    );
                    status.on_play(path, start_paused);
                }
            }
        }
        tracing::info!(session_id = %session_id, "browser worker stopped");
    });
}

fn send_json(sender: &Recipient<BrowserOutbound>, msg: BrowserServerMessage) -> Result<(), ()> {
    let payload = serde_json::to_string(&msg).map_err(|_| ())?;
    sender.do_send(BrowserOutbound(payload));
    Ok(())
}

fn build_stream_url_for(path: &PathBuf, public_base_url: &str) -> String {
    let path_str = path.to_string_lossy();
    let encoded = urlencoding::encode(&path_str);
    format!(
        "{}/stream?path={encoded}",
        public_base_url.trim_end_matches('/')
    )
}
