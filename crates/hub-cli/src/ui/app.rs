use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    widgets::ListState,
    Terminal,
};

use crate::library::{LibraryItem, TrackMeta};
use crate::server_api;
use crate::worker::{self, Command, Event};

use super::render;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanKind {
    Main,
    Preview,
}

struct ScanReq {
    dir: PathBuf,
    kind: ScanKind,
}

struct ScanResp {
    dir: PathBuf,
    kind: ScanKind,
    entries: Result<Vec<LibraryItem>, String>,
}


/// Launch the TUI, spawn the worker thread, and drive the event loop.
pub(crate) fn run_tui(server: String, dir: PathBuf, log_rx: Receiver<String>) -> Result<()> {
    let entries = list_entries_with_parent(&server, &dir)?;
    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    let (evt_tx, evt_rx) = unbounded::<Event>();
    let (scan_tx, scan_rx) = unbounded::<ScanReq>();
    let (scan_done_tx, scan_done_rx) = unbounded::<ScanResp>();
    let active_output_id = Arc::new(Mutex::new(None));
    let evt_tx_worker = evt_tx.clone();
    std::thread::spawn({
        let server = server.clone();
        move || worker::worker_main(server, cmd_rx, evt_tx_worker)
    });

    std::thread::spawn({
        let server = server.clone();
        let evt_tx = evt_tx.clone();
        let active_output_id = active_output_id.clone();
        move || {
            let mut delay = Duration::from_millis(500);
            loop {
                let result = server_api::outputs_stream(&server, |outputs| {
                    if let Ok(mut guard) = active_output_id.lock() {
                        *guard = outputs.active_id.clone();
                    }
                    let _ = evt_tx.send(Event::OutputsUpdate(outputs));
                });
                match result {
                    Ok(_) => delay = Duration::from_millis(500),
                    Err(_) => delay = (delay * 2).min(Duration::from_secs(5)),
                }
                std::thread::sleep(delay);
            }
        }
    });

    std::thread::spawn({
        let server = server.clone();
        let active_output_id = active_output_id.clone();
        let evt_tx = evt_tx.clone();
        move || {
            let mut delay = Duration::from_millis(250);
            loop {
                std::thread::sleep(delay);
                let output_id = active_output_id.lock().ok().and_then(|guard| guard.clone());
                let Some(ref id) = output_id else {
                    delay = (delay * 2).min(Duration::from_secs(2));
                    continue;
                };
                let status_result = {
                    server_api::status_for_output(&server, id)
                };
                match status_result {
                    Ok(status) => {
                        delay = Duration::from_millis(250);
                        if let Some(id) = status.output_id.clone() {
                            if let Ok(mut guard) = active_output_id.lock() {
                                *guard = Some(id);
                            }
                        }
                        if evt_tx
                            .send(Event::RemoteStatus(status))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        delay = (delay * 2).min(Duration::from_secs(2));
                        if let Ok(mut guard) = active_output_id.lock() {
                            *guard = None;
                        }
                    }
                }
            }
        }
    });

    std::thread::spawn({
        let server = server.clone();
        let evt_tx = evt_tx.clone();
        move || {
            let mut delay = Duration::from_millis(500);
            loop {
                std::thread::sleep(delay);
                match server_api::queue_list(&server) {
                    Ok(queue) => {
                        delay = Duration::from_millis(500);
                        let _ = evt_tx.send(Event::QueueUpdate {
                            items: queue.items,
                        });
                    }
                    Err(_) => {
                        delay = (delay * 2).min(Duration::from_secs(2));
                    }
                }
            }
        }
    });

    std::thread::spawn({
        let server = server.clone();
        move || {
            while let Ok(req) = scan_rx.recv() {
                let entries = list_entries_with_parent(&server, &req.dir)
                    .map_err(|e| format!("{e:#}"));
                let _ = scan_done_tx.send(ScanResp {
                    dir: req.dir,
                    kind: req.kind,
                    entries,
                });
            }
        }
    });

    let mut app = App::new(
        server,
        dir,
        entries,
        scan_tx,
        scan_done_rx,
        log_rx,
    );

    let mut term = init_terminal()?;
    let result = ui_loop(&mut term, &mut app, cmd_tx, evt_rx);

    restore_terminal(&mut term)?;
    result
}

/// In-memory UI state for rendering + interaction.
pub(crate) struct App {
    pub(crate) server: String,
    pub(crate) dir: PathBuf,
    pub(crate) entries: Vec<LibraryItem>,
    pub(crate) list_state: ListState,
    pub(crate) now_playing_index: Option<usize>,
    pub(crate) now_playing_path: Option<PathBuf>,
    pub(crate) now_playing_meta: Option<TrackMeta>,
    pub(crate) queued_next: VecDeque<PathBuf>,
    pub(crate) auto_preview: Vec<PathBuf>,
    pub(crate) queue_revision: u64,
    pub(crate) meta_cache: HashMap<PathBuf, TrackMeta>,
    auto_base_path: Option<PathBuf>,
    auto_preview_dirty: bool,
    scan_tx: Sender<ScanReq>,
    scan_rx: Receiver<ScanResp>,
    pending_scan: Option<PathBuf>,
    preview_dir: Option<PathBuf>,
    preview_entries: Vec<LibraryItem>,
    pending_preview_scan: Option<PathBuf>,

    pub(crate) status: String,

    pub(crate) remote_duration_ms: Option<u64>,
    pub(crate) remote_paused: Option<bool>,
    pub(crate) remote_bridge_online: bool,
    pub(crate) remote_elapsed_ms: Option<u64>,
    pub(crate) remote_source_codec: Option<String>,
    pub(crate) remote_source_bit_depth: Option<u16>,
    pub(crate) remote_container: Option<String>,
    pub(crate) remote_output_sample_format: Option<String>,
    pub(crate) remote_resampling: Option<bool>,
    pub(crate) remote_resample_from_hz: Option<u32>,
    pub(crate) remote_resample_to_hz: Option<u32>,
    pub(crate) remote_bitrate_kbps: Option<u32>,
    pub(crate) remote_channels: Option<u16>,
    pub(crate) remote_output_sample_rate: Option<u32>,
    pub(crate) remote_buffered_frames: Option<u64>,
    pub(crate) remote_buffer_capacity_frames: Option<u64>,
    pub(crate) remote_output_id: Option<String>,
    pub(crate) outputs_open: bool,
    pub(crate) outputs: Vec<crate::server_api::RemoteOutput>,
    pub(crate) outputs_active_id: Option<String>,
    pub(crate) outputs_state: ListState,
    pub(crate) outputs_error: Option<String>,

    pub(crate) logs_open: bool,
    pub(crate) logs: VecDeque<String>,
    pub(crate) logs_scroll: usize,
    pub(crate) help_open: bool,
    pub(crate) now_playing_open: bool,
    pub(crate) confirm_clear_queue: bool,
    last_status_snapshot: String,
    log_rx: Receiver<String>,
    pub(crate) list_view_height: usize,
}

impl App {
    fn new(
        server: String,
        dir: PathBuf,
        entries: Vec<LibraryItem>,
        scan_tx: Sender<ScanReq>,
        scan_rx: Receiver<ScanResp>,
        log_rx: Receiver<String>,
    ) -> Self {
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
        }

        let mut meta_cache = HashMap::new();
        for item in &entries {
            if let LibraryItem::Track(track) = item {
                meta_cache.insert(
                    track.path.clone(),
                    TrackMeta {
                        duration_ms: track.duration_ms,
                        sample_rate: track.sample_rate,
                        album: track.album.clone(),
                        artist: track.artist.clone(),
                        format: Some(track.format.clone()),
                    },
                );
            }
        }

        Self {
            server,
            dir,
            entries,
            list_state,
            now_playing_index: None,
            now_playing_path: None,
            now_playing_meta: None,
            queued_next: VecDeque::new(),
            auto_preview: Vec::new(),
            queue_revision: 1,
            meta_cache,
            auto_base_path: None,
            auto_preview_dirty: true,
            scan_tx,
            scan_rx,
            pending_scan: None,
            preview_dir: None,
            preview_entries: Vec::new(),
            pending_preview_scan: None,
            status: "Ready".into(),
            remote_duration_ms: None,
            remote_paused: None,
            remote_bridge_online: false,
            remote_elapsed_ms: None,
            remote_source_codec: None,
            remote_source_bit_depth: None,
            remote_container: None,
            remote_output_sample_format: None,
            remote_resampling: None,
            remote_resample_from_hz: None,
            remote_resample_to_hz: None,
            remote_bitrate_kbps: None,
            remote_channels: None,
            remote_output_sample_rate: None,
            remote_buffered_frames: None,
            remote_buffer_capacity_frames: None,
            remote_output_id: None,
            outputs_open: false,
            outputs: Vec::new(),
            outputs_active_id: None,
            outputs_state: ListState::default(),
            outputs_error: None,
            logs_open: false,
            logs: VecDeque::new(),
            logs_scroll: 0,
            help_open: false,
            now_playing_open: false,
            confirm_clear_queue: false,
            last_status_snapshot: String::new(),
            log_rx,
            list_view_height: 0,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn mark_queue_dirty(&mut self) {
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.auto_preview_dirty = true;
        self.auto_base_path = None;
    }

    fn ensure_output_selected(&mut self) -> bool {
        let Some(output_id) = self.remote_output_id.clone() else {
            self.status = "Select an output first (press o)".into();
            return false;
        };
        if self.remote_bridge_online {
            return true;
        }
        match server_api::outputs_select(&self.server, &output_id) {
            Ok(_) => {
                self.remote_bridge_online = true;
                self.status = "Reconnected output".into();
                true
            }
            Err(e) => {
                self.status = format!("Output offline: {e:#}");
                false
            }
        }
    }

    fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }

    fn toggle_now_playing(&mut self) {
        self.now_playing_open = !self.now_playing_open;
    }

    fn open_clear_queue_confirm(&mut self) {
        self.confirm_clear_queue = true;
    }

    fn close_clear_queue_confirm(&mut self) {
        self.confirm_clear_queue = false;
    }

    fn seek_relative(&mut self, delta_ms: i64, cmd_tx: &Sender<crate::worker::Command>) {
        if !self.ensure_output_selected() {
            return;
        }
        let Some(elapsed) = self.remote_elapsed_ms else {
            self.status = "Seek unavailable (no progress yet)".into();
            return;
        };
        let mut target = if delta_ms >= 0 {
            elapsed.saturating_add(delta_ms as u64)
        } else {
            elapsed.saturating_sub(delta_ms.unsigned_abs())
        };
        if let Some(total) = self.remote_duration_ms {
            if target > total {
                target = total;
            }
        }
        cmd_tx.send(crate::worker::Command::Seek { ms: target }).ok();
    }

    pub(crate) fn refresh_auto_preview_if_needed(&mut self) {
        if !self.auto_preview_dirty {
            return;
        }

        let base = self
            .now_playing_path
            .clone()
            .or_else(|| self.queued_next.back().cloned());

        let Some(base) = base else {
            // Keep the existing preview if there's no active base yet.
            self.auto_preview_dirty = false;
            return;
        };
        self.auto_base_path = Some(base.clone());
        let Some(parent) = base.parent() else {
            self.auto_preview_dirty = false;
            return;
        };
        let entries = if parent == self.dir.as_path() {
            &self.entries
        } else {
            let preview_ready = self
                .preview_dir
                .as_ref()
                .map(|d| d.as_path() == parent)
                .unwrap_or(false);
            if !preview_ready {
                let _ = self.request_preview_scan(parent.to_path_buf());
                return;
            }
            &self.preview_entries
        };
        let queue: Vec<PathBuf> = entries
            .iter()
            .filter_map(|item| match item {
                LibraryItem::Track(t) => Some(t.path.clone()),
                _ => None,
            })
            .collect();
        let mut next = Vec::new();
        if let Some(idx) = queue.iter().position(|p| p == &base) {
            for path in queue.iter().skip(idx + 1) {
                next.push(path.clone());
            }
        }
        self.auto_preview = next;
        self.auto_preview_dirty = false;
    }

    // metadata is provided by the server; no local probing needed.

    fn request_scan(&mut self, dir: PathBuf) -> Result<()> {
        self.pending_scan = Some(dir.clone());
        self.preview_dir = None;
        self.preview_entries.clear();
        self.pending_preview_scan = None;
        self.scan_tx
            .send(ScanReq { dir, kind: ScanKind::Main })
            .map_err(|_| anyhow::anyhow!("scan thread is not available"))?;
        Ok(())
    }

    fn request_preview_scan(&mut self, dir: PathBuf) -> Result<()> {
        if self.pending_preview_scan.as_ref() == Some(&dir) {
            return Ok(());
        }
        self.pending_preview_scan = Some(dir.clone());
        self.scan_tx
            .send(ScanReq { dir, kind: ScanKind::Preview })
            .map_err(|_| anyhow::anyhow!("scan thread is not available"))?;
        Ok(())
    }

    fn drain_scan_results(&mut self) {
        while let Ok(resp) = self.scan_rx.try_recv() {
            match resp.kind {
                ScanKind::Main => {
                    let Some(pending) = self.pending_scan.as_ref() else {
                        continue;
                    };
                    if &resp.dir != pending {
                        continue;
                    }
                    self.pending_scan = None;
                    match resp.entries {
                        Ok(entries) => {
                            self.entries = entries;
                            for item in &self.entries {
                                if let LibraryItem::Track(track) = item {
                                    self.meta_cache.insert(
                                        track.path.clone(),
                                        TrackMeta {
                                            duration_ms: track.duration_ms,
                                            sample_rate: track.sample_rate,
                                            album: track.album.clone(),
                                            artist: track.artist.clone(),
                                            format: Some(track.format.clone()),
                                        },
                                    );
                                }
                            }
                            self.now_playing_index = self
                                .now_playing_path
                                .as_ref()
                                .and_then(|path| {
                                    self.entries.iter().position(|item| item.path() == path)
                                });
                            if self.entries.is_empty() {
                                self.list_state.select(None);
                            } else {
                                self.list_state.select(Some(0));
                            }
                            if let Some(idx) = self.now_playing_index {
                                self.select_index(idx);
                            }
                            self.status = format!("Entered {:?}", self.dir);
                        }
                        Err(e) => {
                            self.status = format!("Scan error: {e}");
                        }
                    }
                }
                ScanKind::Preview => {
                    if self.pending_preview_scan.as_ref() != Some(&resp.dir) {
                        continue;
                    }
                    self.pending_preview_scan = None;
                    match resp.entries {
                        Ok(entries) => {
                            self.preview_dir = Some(resp.dir);
                            self.preview_entries = entries;
                            for item in &self.preview_entries {
                                if let LibraryItem::Track(track) = item {
                                    self.meta_cache.insert(
                                        track.path.clone(),
                                        TrackMeta {
                                            duration_ms: track.duration_ms,
                                            sample_rate: track.sample_rate,
                                            album: track.album.clone(),
                                            artist: track.artist.clone(),
                                            format: Some(track.format.clone()),
                                        },
                                    );
                                }
                            }
                        }
                        Err(_) => {
                            self.preview_dir = None;
                            self.preview_entries.clear();
                        }
                    }
                }
            }
        }
    }

    fn selected_dir(&self) -> Option<PathBuf> {
        self.selected_index()
            .and_then(|i| self.entries.get(i))
            .and_then(|entry| match entry {
                LibraryItem::Dir { path, .. } => Some(path.clone()),
                _ => None,
            })
    }

    fn select_next(&mut self) {
        if self.outputs_open {
            if self.outputs.is_empty() {
                return;
            }
            let i = self.outputs_state.selected().unwrap_or(0);
            let ni = (i + 1).min(self.outputs.len() - 1);
            self.outputs_state.select(Some(ni));
            return;
        }
        if self.entries.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = (i + 1).min(self.entries.len() - 1);
        self.list_state.select(Some(ni));
    }

    fn select_prev(&mut self) {
        if self.outputs_open {
            if self.outputs.is_empty() {
                return;
            }
            let i = self.outputs_state.selected().unwrap_or(0);
            let ni = i.saturating_sub(1);
            self.outputs_state.select(Some(ni));
            return;
        }
        if self.entries.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = i.saturating_sub(1);
        self.list_state.select(Some(ni));
    }

    fn page_step(&self) -> usize {
        self.list_view_height.max(1)
    }

    fn page_down(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let step = self.page_step();
        let i = self.selected_index().unwrap_or(0);
        let ni = (i + step).min(self.entries.len() - 1);
        self.list_state.select(Some(ni));
    }

    fn page_up(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let step = self.page_step();
        let i = self.selected_index().unwrap_or(0);
        let ni = i.saturating_sub(step);
        self.list_state.select(Some(ni));
    }

    fn rescan(&mut self) -> Result<()> {
        if let Err(e) = server_api::rescan(&self.server) {
            self.status = format!("Rescan request failed: {e:#}");
        }
        self.request_scan(self.dir.clone())?;
        Ok(())
    }

    fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.dir.parent() {
            self.dir = parent.to_path_buf();
            self.request_scan(self.dir.clone())?;
            self.status = format!("Entering {:?}", self.dir);
        }
        Ok(())
    }

    fn enter_dir(&mut self, dir: PathBuf) -> Result<()> {
        self.dir = dir;
        self.request_scan(self.dir.clone())?;
        self.status = format!("Entering {:?}", self.dir);
        Ok(())
    }

    fn jump_to_playing(&mut self) -> Result<()> {
        let Some(path) = self.now_playing_path.clone() else {
            self.status = "Nothing playing".into();
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            self.status = "Playing track has no parent".into();
            return Ok(());
        };
        self.dir = parent.to_path_buf();
        self.request_scan(self.dir.clone())?;
        self.status = "Jumping to playing".into();
        Ok(())
    }

    fn select_index(&mut self, idx: usize) {
        self.list_state.select(Some(idx));
    }

    fn open_outputs(&mut self) {
        match server_api::outputs(&self.server) {
            Ok(resp) => {
                self.outputs = resp.outputs;
                self.outputs_active_id = resp.active_id;
                self.outputs_state.select(Some(0));
                if let Some(active) = self.outputs_active_id.as_ref() {
                    if let Some(idx) = self.outputs.iter().position(|o| &o.id == active) {
                        self.outputs_state.select(Some(idx));
                    }
                }
                self.outputs_error = None;
                self.outputs_open = true;
                self.status = "Select output".into();
            }
            Err(e) => {
                self.outputs_error = Some(format!("{e:#}"));
                self.status = "Failed to load outputs".into();
            }
        }
    }

    fn close_outputs(&mut self) {
        self.outputs_open = false;
    }

    fn toggle_logs(&mut self) {
        self.logs_open = !self.logs_open;
        if !self.logs_open {
            self.logs_scroll = 0;
        }
    }

    fn scroll_logs_up(&mut self) {
        let max = self.logs.len().saturating_sub(1);
        self.logs_scroll = (self.logs_scroll + 1).min(max);
    }

    fn scroll_logs_down(&mut self) {
        self.logs_scroll = self.logs_scroll.saturating_sub(1);
    }

    fn push_log_line(&mut self, line: String) {
        const LOG_CAP: usize = 500;
        if self.logs.len() >= LOG_CAP {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn note_status_change(&mut self) {
        if self.last_status_snapshot == self.status {
            return;
        }
        let line = self.status.clone();
        self.last_status_snapshot = self.status.clone();
        tracing::info!("{line}");
    }

    fn drain_logs(&mut self) {
        while let Ok(line) = self.log_rx.try_recv() {
            self.push_log_line(line);
        }
    }

    fn select_output(&mut self) {
        let Some(idx) = self.outputs_state.selected() else {
            return;
        };
        let Some(output) = self.outputs.get(idx) else {
            return;
        };
        if let Err(e) = server_api::outputs_select(&self.server, &output.id) {
            self.status = format!("Output select failed: {e:#}");
            return;
        }
        self.outputs_active_id = Some(output.id.clone());
        self.remote_output_id = Some(output.id.clone());
        self.remote_bridge_online = true;
        self.outputs_open = false;
        self.status = format!("Output set: {}", output.name);
    }

    fn toggle_queue_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(LibraryItem::Track(track)) = self.entries.get(index) else {
            self.status = "Cannot queue a folder".into();
            return;
        };
        let track_path = track.path.clone();

        if let Some(pos) = self.queued_next.iter().position(|p| p == &track_path) {
            if server_api::queue_remove(&self.server, &track_path).is_ok() {
                self.queued_next.remove(pos);
                self.status = "Unqueued".into();
                self.mark_queue_dirty();
            } else {
                self.status = "Unqueue failed".into();
            }
        } else if server_api::queue_add(&self.server, &[track_path.clone()]).is_ok() {
            self.queued_next.push_back(track_path.clone());
            self.status = "Queued".into();
            self.mark_queue_dirty();
        } else {
            self.status = "Queue failed".into();
        }
    }

    fn queue_next_selected(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(LibraryItem::Track(track)) = self.entries.get(index) else {
            self.status = "Cannot queue a folder".into();
            return;
        };
        if server_api::queue_add_next(&self.server, &[track.path.clone()]).is_ok() {
            self.status = "Queued next".into();
            self.mark_queue_dirty();
        } else {
            self.status = "Queue next failed".into();
        }
    }

    fn queue_all_current_dir(&mut self) {
        let paths: Vec<PathBuf> = self
            .entries
            .iter()
            .filter_map(|item| match item {
                LibraryItem::Track(t) => Some(t.path.clone()),
                _ => None,
            })
            .collect();
        if paths.is_empty() {
            self.status = "No tracks to queue".into();
            return;
        }
        match server_api::queue_add(&self.server, &paths) {
            Ok(_) => {
                self.status = format!("Queued {} tracks", paths.len());
            }
            Err(e) => {
                self.status = format!("Queue failed: {e:#}");
            }
        }
    }

    fn play_track_at(&mut self, index: usize, _cmd_tx: &Sender<Command>) {
        if !self.ensure_output_selected() {
            return;
        }
        let Some(LibraryItem::Track(track)) = self.entries.get(index) else {
            return;
        };
        let track_path = track.path.clone();
        let track_meta = TrackMeta {
            duration_ms: track.duration_ms,
            sample_rate: track.sample_rate,
            album: track.album.clone(),
            artist: track.artist.clone(),
            format: Some(track.format.clone()),
        };
        let track_duration = track.duration_ms;

        if server_api::play_keep(&self.server, &track_path).is_err() {
            self.status = "Play failed".into();
        }
        self.now_playing_index = Some(index);
        self.now_playing_path = Some(track_path.clone());
        self.now_playing_meta = Some(track_meta);
        self.remote_duration_ms = track_duration;
        self.remote_elapsed_ms = Some(0);
        self.remote_paused = Some(false);
        self.mark_queue_dirty();
    }

}

fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    cmd_tx: Sender<Command>,
    evt_rx: Receiver<Event>,
) -> Result<()> {
    let tick = Duration::from_millis(33);
    let mut last_tick = Instant::now();

    loop {
        // Pump worker events.
        while let Ok(ev) = evt_rx.try_recv() {
            match ev {
                Event::Status(s) => app.status = s,
                Event::OutputsUpdate(outputs) => {
                    app.outputs = outputs.outputs;
                    app.outputs_active_id = outputs.active_id;
                    app.outputs_error = None;
                    if app.outputs_open {
                        if let Some(active_id) = app.outputs_active_id.as_ref() {
                            if let Some(idx) = app.outputs.iter().position(|o| &o.id == active_id) {
                                app.outputs_state.select(Some(idx));
                            }
                        } else if app.outputs.is_empty() {
                            app.outputs_state.select(None);
                        } else {
                            let selected = app.outputs_state.selected().unwrap_or(0);
                            let next = selected.min(app.outputs.len() - 1);
                            app.outputs_state.select(Some(next));
                        }
                    }
                }
                Event::QueueUpdate { items } => {
                    app.queued_next = items.iter().map(|item| item.path.clone()).collect();
                    for item in items {
                        if let Some(meta) = item.meta {
                            app.meta_cache.insert(item.path.clone(), meta);
                        }
                    }
                    app.mark_queue_dirty();
                }
                Event::RemoteStatus(status) => {
                    let now_playing = status.now_playing.clone();
                    let elapsed_ms = status.elapsed_ms;
                    let duration_ms = status.duration_ms;
                    let paused = status.paused;
                    let bridge_online = status.bridge_online;
                    let source_codec = status.source_codec.clone();
                    let source_bit_depth = status.source_bit_depth;
                    let container = status.container.clone();
                    let output_sample_format = status.output_sample_format.clone();
                    let resampling = status.resampling;
                    let resample_from_hz = status.resample_from_hz;
                    let resample_to_hz = status.resample_to_hz;
                    let sample_rate = status.sample_rate;
                    let channels = status.channels;
                    let output_sample_rate = status.output_sample_rate;
                    let title = status.title.clone();
                    let artist = status.artist.clone();
                    let album = status.album.clone();
                    let format = status.format.clone();
                    let output_id = status.output_id.clone();
                    let bitrate_kbps = status.bitrate_kbps;
                    let buffered_frames = status.buffered_frames;
                    let buffer_capacity_frames = status.buffer_capacity_frames;
                    if let Some(path) = now_playing {
                        let path = PathBuf::from(path);
                        app.now_playing_path = Some(path.clone());
                        app.now_playing_index = app.entries.iter().position(|item| item.path() == path);
                        app.mark_queue_dirty();
                    } else {
                        app.now_playing_path = None;
                        app.now_playing_index = None;
                        app.now_playing_meta = None;
                        app.remote_channels = None;
                        app.remote_output_sample_rate = None;
                        app.remote_buffered_frames = None;
                        app.remote_buffer_capacity_frames = None;
                    }
                    if duration_ms.is_some() {
                        app.remote_duration_ms = duration_ms;
                    }
                    app.remote_elapsed_ms = elapsed_ms;
                    app.remote_paused = Some(paused);
                    app.remote_bridge_online = bridge_online;
                    app.remote_source_codec = source_codec;
                    app.remote_source_bit_depth = source_bit_depth;
                    app.remote_container = container;
                    app.remote_output_sample_format = output_sample_format;
                    app.remote_resampling = resampling;
                    app.remote_resample_from_hz = resample_from_hz;
                    app.remote_resample_to_hz = resample_to_hz;
                    app.remote_bitrate_kbps = bitrate_kbps;
                    if title.is_some() || artist.is_some() || album.is_some() || format.is_some() {
                        let mut meta = app.now_playing_meta.clone().unwrap_or_default();
                        if artist.is_some() {
                            meta.artist = artist;
                        }
                        if album.is_some() {
                            meta.album = album;
                        }
                        if format.is_some() {
                            meta.format = format;
                        }
                        if duration_ms.is_some() {
                            meta.duration_ms = duration_ms;
                        }
                        app.now_playing_meta = Some(meta);
                    }
                    if let Some(sr) = sample_rate {
                        let mut meta = app.now_playing_meta.clone().unwrap_or_default();
                        meta.sample_rate = Some(sr);
                        app.now_playing_meta = Some(meta);
                    }
                    app.remote_channels = channels;
                    app.remote_output_sample_rate = output_sample_rate;
                    app.remote_buffered_frames = buffered_frames;
                    app.remote_buffer_capacity_frames = buffer_capacity_frames;
                    app.remote_output_id = output_id;
                }
                Event::Error(e) => app.status = format!("Error: {e}"),
            }
        }

        app.drain_scan_results();

        app.drain_logs();
        terminal.draw(|f| render::draw(f, app))?;

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).context("poll terminal events")? {
            if let CEvent::Key(k) = event::read().context("read terminal event")? {
                if app.logs_open {
                    match k.code {
                        KeyCode::Char('q') => {
                            cmd_tx.send(Command::Quit).ok();
                            return Ok(());
                        }
                        KeyCode::Esc | KeyCode::Char('l') => app.toggle_logs(),
                        KeyCode::Up => app.scroll_logs_up(),
                        KeyCode::Down => app.scroll_logs_down(),
                        _ => {}
                    }
                    continue;
                }
                if app.help_open {
                    match k.code {
                        KeyCode::Char('q') => {
                            cmd_tx.send(Command::Quit).ok();
                            return Ok(());
                        }
                        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?') => app.toggle_help(),
                        _ => {}
                    }
                    continue;
                }
                if app.now_playing_open {
                    match k.code {
                        KeyCode::Char('q') => {
                            cmd_tx.send(Command::Quit).ok();
                            return Ok(());
                        }
                        KeyCode::Esc | KeyCode::Char('i') => app.toggle_now_playing(),
                        _ => {}
                    }
                    continue;
                }
                if app.confirm_clear_queue {
                    match k.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            cmd_tx.send(Command::QueueClear).ok();
                            app.close_clear_queue_confirm();
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app.close_clear_queue_confirm();
                        }
                        _ => {}
                    }
                    continue;
                }
                if app.outputs_open {
                    match k.code {
                        KeyCode::Esc => app.close_outputs(),
                        KeyCode::Up => app.select_prev(),
                        KeyCode::Down => app.select_next(),
                        KeyCode::Enter => app.select_output(),
                        _ => {}
                    }
                    continue;
                }
                let shift = k.modifiers.contains(KeyModifiers::SHIFT);
                match k.code {
                    KeyCode::Char('q') => {
                        cmd_tx.send(Command::Quit).ok();
                        return Ok(());
                    }
                    KeyCode::Up => app.select_prev(),
                    KeyCode::Down => app.select_next(),
                    KeyCode::PageUp => app.page_up(),
                    KeyCode::PageDown => app.page_down(),
                    KeyCode::Left => {
                        let delta = if shift { -30_000 } else { -5_000 };
                        app.seek_relative(delta, &cmd_tx);
                    }
                    KeyCode::Right => {
                        let delta = if shift { 30_000 } else { 5_000 };
                        app.seek_relative(delta, &cmd_tx);
                    }
                    KeyCode::Backspace => {
                        app.go_parent()?;
                    }
                    KeyCode::Char('r') => {
                        app.rescan()?;
                        app.status = "Rescanning...".into();
                    }
                    KeyCode::Enter => {
                        if let Some(dir) = app.selected_dir() {
                            app.enter_dir(dir)?;
                        } else if let Some(index) = app.selected_index() {
                            app.play_track_at(index, &cmd_tx);
                        }
                    }
                    KeyCode::Char(' ') => {
                        cmd_tx.send(Command::PauseToggle).ok();
                    }
                    KeyCode::Char('s') => {
                        cmd_tx.send(Command::Stop).ok();
                        app.status = "Stopped".into();
                    }
                    KeyCode::Char('c') => {
                        app.open_clear_queue_confirm();
                    }
                    KeyCode::Char('n') => {
                        if app.ensure_output_selected() {
                            cmd_tx.send(Command::Next).ok();
                            app.status = "Skipping".into();
                        }
                    }
                    KeyCode::Char('k') => {
                        app.toggle_queue_selected();
                    }
                    KeyCode::Char('m') => {
                        app.queue_next_selected();
                    }
                    KeyCode::Char('K') => {
                        app.queue_all_current_dir();
                    }
                    KeyCode::Char('p') => {
                        app.jump_to_playing()?;
                    }
                    KeyCode::Char('o') => {
                        app.open_outputs();
                    }
                    KeyCode::Char('l') => {
                        app.toggle_logs();
                    }
                    KeyCode::Char('i') => {
                        app.toggle_now_playing();
                    }
                    KeyCode::Char('h') | KeyCode::Char('?') => {
                        app.toggle_help();
                    }
                    _ => {}
                }
            }
        }

        app.note_status_change();

        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }
    }
}

fn list_entries_with_parent(server: &str, dir: &PathBuf) -> Result<Vec<LibraryItem>> {
    let mut entries = server_api::list_entries(server, dir)?;
    if let Some(parent) = dir.parent() {
        entries.insert(
            0,
            LibraryItem::Dir {
                path: parent.to_path_buf(),
                name: "..".to_string(),
            },
        );
    }
    Ok(entries)
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("create terminal")?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Track;
    use crate::worker::Command;
    use crossbeam_channel::unbounded;

    fn track_item(path: &str, artist: &str) -> LibraryItem {
        LibraryItem::Track(Track {
            path: PathBuf::from(path),
            file_name: path
                .rsplit('/')
                .next()
                .unwrap_or("file.flac")
                .to_string(),
            duration_ms: Some(123_000),
            sample_rate: Some(48_000),
            album: Some("Album".to_string()),
            artist: Some(artist.to_string()),
            format: "FLAC".to_string(),
        })
    }

    fn app_with_entries(entries: Vec<LibraryItem>) -> App {
        let (scan_tx, _scan_req_rx) = unbounded::<ScanReq>();
        let (_scan_done_tx, scan_rx) = unbounded::<ScanResp>();
        let (_log_tx, log_rx) = unbounded::<String>();
        App::new(
            "http://127.0.0.1:8080".to_string(),
            PathBuf::from("/music"),
            entries,
            scan_tx,
            scan_rx,
            log_rx,
        )
    }

    fn app_with_entries_and_logs(entries: Vec<LibraryItem>) -> (App, Sender<String>) {
        let (scan_tx, _scan_req_rx) = unbounded::<ScanReq>();
        let (_scan_done_tx, scan_rx) = unbounded::<ScanResp>();
        let (log_tx, log_rx) = unbounded::<String>();
        (
            App::new(
                "http://127.0.0.1:8080".to_string(),
                PathBuf::from("/music"),
                entries,
                scan_tx,
                scan_rx,
                log_rx,
            ),
            log_tx,
        )
    }

    #[test]
    fn meta_cache_seeded_from_entries() {
        let entries = vec![
            track_item("/music/a.flac", "Artist A"),
            track_item("/music/b.flac", "Artist B"),
        ];
        let app = app_with_entries(entries);
        let a = app.meta_cache.get(&PathBuf::from("/music/a.flac")).unwrap();
        assert_eq!(a.artist.as_deref(), Some("Artist A"));
    }

    #[test]
    fn auto_preview_uses_now_playing_base() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
            track_item("/music/c.flac", "C"),
        ];
        let mut app = app_with_entries(entries);
        app.now_playing_path = Some(PathBuf::from("/music/b.flac"));
        app.mark_queue_dirty();
        app.refresh_auto_preview_if_needed();
        assert_eq!(app.auto_preview, vec![PathBuf::from("/music/c.flac")]);
    }

    #[test]
    fn request_scan_does_not_clear_auto_preview() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.auto_preview = vec![PathBuf::from("/music/next.flac")];
        app.pending_scan = Some(PathBuf::from("/music/other"));
        assert_eq!(app.auto_preview, vec![PathBuf::from("/music/next.flac")]);
    }

    #[test]
    fn mark_queue_dirty_resets_auto_preview_state() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        let revision = app.queue_revision;
        app.auto_preview_dirty = false;
        app.auto_base_path = Some(PathBuf::from("/music/a.flac"));
        app.mark_queue_dirty();
        assert_ne!(app.queue_revision, revision);
        assert!(app.auto_preview_dirty);
        assert!(app.auto_base_path.is_none());
    }

    #[test]
    fn seek_relative_clamps_to_duration() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        let (tx, rx) = unbounded::<Command>();
        app.remote_output_id = Some("bridge:test:device".to_string());
        app.remote_bridge_online = true;
        app.remote_elapsed_ms = Some(90_000);
        app.remote_duration_ms = Some(100_000);

        app.seek_relative(20_000, &tx);

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(cmd, Command::Seek { ms: 100_000 }));
    }

    #[test]
    fn seek_relative_handles_missing_progress() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        let (tx, rx) = unbounded::<Command>();
        app.remote_output_id = Some("bridge:test:device".to_string());
        app.remote_bridge_online = true;
        app.remote_elapsed_ms = None;

        app.seek_relative(5_000, &tx);

        assert!(rx.try_recv().is_err());
        assert_eq!(app.status, "Seek unavailable (no progress yet)");
    }

    #[test]
    fn toggle_logs_resets_scroll_when_closed() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.logs_open = true;
        app.logs_scroll = 5;
        app.toggle_logs();
        assert!(!app.logs_open);
        assert_eq!(app.logs_scroll, 0);
    }

    #[test]
    fn scroll_logs_up_clamps_to_len() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.logs.push_back("line1".to_string());
        app.logs.push_back("line2".to_string());
        app.logs_scroll = 0;
        app.scroll_logs_up();
        app.scroll_logs_up();
        assert_eq!(app.logs_scroll, 1);
    }

    #[test]
    fn scroll_logs_down_stops_at_zero() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.logs_scroll = 2;
        app.scroll_logs_down();
        app.scroll_logs_down();
        app.scroll_logs_down();
        assert_eq!(app.logs_scroll, 0);
    }

    #[test]
    fn select_next_clamps_at_end() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
        ];
        let mut app = app_with_entries(entries);
        app.select_index(1);
        app.select_next();
        assert_eq!(app.selected_index(), Some(1));
    }

    #[test]
    fn select_prev_clamps_at_start() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
        ];
        let mut app = app_with_entries(entries);
        app.select_index(0);
        app.select_prev();
        assert_eq!(app.selected_index(), Some(0));
    }

    #[test]
    fn page_step_returns_list_height_with_minimum() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.list_view_height = 0;
        assert_eq!(app.page_step(), 1);
        app.list_view_height = 10;
        assert_eq!(app.page_step(), 10);
    }

    #[test]
    fn page_down_advances_by_step() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
            track_item("/music/c.flac", "C"),
            track_item("/music/d.flac", "D"),
        ];
        let mut app = app_with_entries(entries);
        app.list_view_height = 3;
        app.select_index(0);
        app.page_down();
        assert_eq!(app.selected_index(), Some(3));
    }

    #[test]
    fn page_up_moves_back_by_step() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
            track_item("/music/c.flac", "C"),
            track_item("/music/d.flac", "D"),
        ];
        let mut app = app_with_entries(entries);
        app.list_view_height = 3;
        app.select_index(3);
        app.page_up();
        assert_eq!(app.selected_index(), Some(0));
    }

    #[test]
    fn drain_logs_moves_lines_into_buffer() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let (mut app, log_tx) = app_with_entries_and_logs(entries);
        log_tx.send("line1".to_string()).unwrap();
        log_tx.send("line2".to_string()).unwrap();

        app.drain_logs();

        assert_eq!(app.logs.len(), 2);
        assert_eq!(app.logs[0], "line1");
        assert_eq!(app.logs[1], "line2");
    }

    #[test]
    fn push_log_line_caps_buffer_size() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        for i in 0..600 {
            app.push_log_line(format!("line-{i}"));
        }
        assert!(app.logs.len() <= 500);
        assert_eq!(app.logs.back().unwrap(), "line-599");
    }

    #[test]
    fn toggle_now_playing_flips_flag() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        assert!(!app.now_playing_open);
        app.toggle_now_playing();
        assert!(app.now_playing_open);
    }

    // auto-advance moved to server; client no longer manipulates queue on playback end.
}
