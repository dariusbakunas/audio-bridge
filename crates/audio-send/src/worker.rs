//! Background worker that forwards control commands to the HTTP server.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender};

use crate::server_api;

/// Commands sent from the UI thread to the worker.
#[derive(Debug, Clone)]
pub enum Command {
    Play { path: PathBuf },
    PauseToggle,
    Next,
    Quit,
}

/// Events sent from the worker back to the UI thread.
#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    RemoteStatus {
        now_playing: Option<String>,
        elapsed_ms: Option<u64>,
        duration_ms: Option<u64>,
        paused: bool,
        sample_rate: Option<u32>,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        format: Option<String>,
    },
    Error(String),
}

pub fn worker_main(server: String, cmd_rx: Receiver<Command>, evt_tx: Sender<Event>) {
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Quit => break,
            Command::Play { path } => {
                if let Err(e) = server_api::play(&server, &path) {
                    let _ = evt_tx.send(Event::Error(format!("Play failed: {e:#}")));
                } else {
                    let _ = evt_tx.send(Event::Status("Playing".into()));
                }
            }
            Command::PauseToggle => {
                if let Err(e) = server_api::pause_toggle(&server) {
                    let _ = evt_tx.send(Event::Error(format!("Pause failed: {e:#}")));
                } else {
                    let _ = evt_tx.send(Event::Status("Toggled pause".into()));
                }
            }
            Command::Next => {
                if let Err(e) = server_api::next(&server) {
                    let _ = evt_tx.send(Event::Error(format!("Next failed: {e:#}")));
                } else {
                    let _ = evt_tx.send(Event::Status("Next".into()));
                }
            }
        }
    }
}
