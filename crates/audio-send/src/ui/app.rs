use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, Sender};
use crossterm::{
    event::{self, Event as CEvent, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    widgets::ListState,
    Terminal,
};

use crate::library::{self, LibraryItem, Track, TrackMeta};
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

struct MetaReq {
    path: PathBuf,
    ext_hint: String,
}

struct MetaResp {
    path: PathBuf,
    meta: TrackMeta,
}

/// Launch the TUI, spawn the worker thread, and drive the event loop.
pub(crate) fn run_tui(addr: SocketAddr, dir: PathBuf) -> Result<()> {
    let entries = list_entries_with_parent(&dir)?;
    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    let (evt_tx, evt_rx) = unbounded::<Event>();
    let (scan_tx, scan_rx) = unbounded::<ScanReq>();
    let (scan_done_tx, scan_done_rx) = unbounded::<ScanResp>();
    let (meta_tx, meta_rx) = unbounded::<MetaReq>();
    let (meta_done_tx, meta_done_rx) = unbounded::<MetaResp>();

    std::thread::spawn({
        let addr = addr;
        move || worker::worker_main(addr, cmd_rx, evt_tx)
    });

    std::thread::spawn(move || {
        while let Ok(req) = meta_rx.recv() {
            let meta = library::probe_track_meta(&req.path, &req.ext_hint);
            let _ = meta_done_tx.send(MetaResp {
                path: req.path,
                meta,
            });
        }
    });

    std::thread::spawn(move || {
        while let Ok(req) = scan_rx.recv() {
            let entries = list_entries_with_parent(&req.dir)
                .map_err(|e| format!("{e:#}"));
            let _ = scan_done_tx.send(ScanResp {
                dir: req.dir,
                kind: req.kind,
                entries,
            });
        }
    });

    let mut app = App::new(
        addr,
        dir,
        entries,
        scan_tx,
        scan_done_rx,
        meta_tx,
        meta_done_rx,
    );

    let mut term = init_terminal()?;
    let result = ui_loop(&mut term, &mut app, cmd_tx, evt_rx);

    restore_terminal(&mut term)?;
    result
}

/// In-memory UI state for rendering + interaction.
pub(crate) struct App {
    pub(crate) addr: SocketAddr,
    pub(crate) dir: PathBuf,
    pub(crate) entries: Vec<LibraryItem>,
    pub(crate) list_state: ListState,
    pub(crate) now_playing_index: Option<usize>,
    pub(crate) now_playing_path: Option<PathBuf>,
    pub(crate) now_playing_meta: Option<TrackMeta>,
    pub(crate) playing_queue: Vec<PathBuf>,
    pub(crate) playing_queue_index: Option<usize>,
    pub(crate) queued_next: VecDeque<PathBuf>,
    pub(crate) auto_preview: Vec<PathBuf>,
    pub(crate) queue_revision: u64,
    pub(crate) auto_preview_revision: u64,
    pub(crate) meta_cache: HashMap<PathBuf, TrackMeta>,
    auto_base_path: Option<PathBuf>,
    auto_preview_dirty: bool,
    scan_tx: Sender<ScanReq>,
    scan_rx: Receiver<ScanResp>,
    pending_scan: Option<PathBuf>,
    preview_dir: Option<PathBuf>,
    preview_entries: Vec<LibraryItem>,
    pending_preview_scan: Option<PathBuf>,
    meta_tx: Sender<MetaReq>,
    meta_rx: Receiver<MetaResp>,
    meta_inflight: HashSet<PathBuf>,
    pub(crate) auto_advance_armed: bool,

    pub(crate) status: String,
    pub(crate) last_progress: Option<(u64, Option<u64>)>, // sent, total

    pub(crate) remote_sample_rate: Option<u32>,
    pub(crate) remote_channels: Option<u16>,
    pub(crate) remote_duration_ms: Option<u64>,
    pub(crate) remote_played_frames: Option<u64>,
    pub(crate) remote_paused: Option<bool>,
}

impl App {
    pub(crate) fn new(
        addr: SocketAddr,
        dir: PathBuf,
        entries: Vec<LibraryItem>,
        scan_tx: Sender<ScanReq>,
        scan_rx: Receiver<ScanResp>,
        meta_tx: Sender<MetaReq>,
        meta_rx: Receiver<MetaResp>,
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
            addr,
            dir,
            entries,
            list_state,
            now_playing_index: None,
            now_playing_path: None,
            now_playing_meta: None,
            playing_queue: Vec::new(),
            playing_queue_index: None,
            queued_next: VecDeque::new(),
            auto_preview: Vec::new(),
            queue_revision: 1,
            auto_preview_revision: 0,
            meta_cache,
            auto_base_path: None,
            auto_preview_dirty: true,
            scan_tx,
            scan_rx,
            pending_scan: None,
            preview_dir: None,
            preview_entries: Vec::new(),
            pending_preview_scan: None,
            meta_tx,
            meta_rx,
            meta_inflight: HashSet::new(),
            auto_advance_armed: false,
            status: "Ready".into(),
            last_progress: None,
            remote_sample_rate: None,
            remote_channels: None,
            remote_duration_ms: None,
            remote_played_frames: None,
            remote_paused: None,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn selected_track(&self) -> Option<&Track> {
        self.selected_index().and_then(|i| match self.entries.get(i) {
            Some(LibraryItem::Track(track)) => Some(track),
            _ => None,
        })
    }

    fn mark_queue_dirty(&mut self) {
        self.queue_revision = self.queue_revision.wrapping_add(1);
        self.auto_preview_dirty = true;
        self.auto_base_path = None;
    }

    pub(crate) fn refresh_auto_preview_if_needed(&mut self) {
        if !self.auto_preview_dirty {
            return;
        }

        let base = self
            .queued_next
            .back()
            .cloned()
            .or_else(|| self.now_playing_path.clone());

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

    pub(crate) fn ensure_meta_for_path(&mut self, path: &PathBuf) {
        if self.meta_cache.contains_key(path) || self.meta_inflight.contains(path) {
            return;
        }
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if self
            .meta_tx
            .send(MetaReq {
                path: path.clone(),
                ext_hint,
            })
            .is_ok()
        {
            self.meta_inflight.insert(path.clone());
        }
    }

    fn drain_meta_results(&mut self) {
        while let Ok(resp) = self.meta_rx.try_recv() {
            self.meta_inflight.remove(&resp.path);
            if self.now_playing_path.as_ref() == Some(&resp.path) {
                self.now_playing_meta = Some(resp.meta.clone());
            }
            self.meta_cache.insert(resp.path, resp.meta);
        }
    }

    fn request_scan(&mut self, dir: PathBuf) -> Result<()> {
        self.pending_scan = Some(dir.clone());
        self.meta_inflight.clear();
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
        if self.entries.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = (i + 1).min(self.entries.len() - 1);
        self.list_state.select(Some(ni));
    }

    fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = i.saturating_sub(1);
        self.list_state.select(Some(ni));
    }

    fn rescan(&mut self) -> Result<()> {
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
            self.queued_next.remove(pos);
            self.status = "Unqueued".into();
            self.mark_queue_dirty();
        } else {
            self.queued_next.push_back(track_path.clone());
            self.ensure_meta_for_path(&track_path);
            self.status = "Queued".into();
            self.mark_queue_dirty();
        }
    }

    fn next_track_index_from(&self, from: usize) -> Option<usize> {
        if from >= self.entries.len() {
            return None;
        }
        for i in (from + 1)..self.entries.len() {
            if matches!(self.entries.get(i), Some(LibraryItem::Track(_))) {
                return Some(i);
            }
        }
        None
    }

    fn play_track_at(&mut self, index: usize, cmd_tx: &Sender<Command>) {
        let Some(LibraryItem::Track(track)) = self.entries.get(index) else {
            return;
        };
        let track_path = track.path.clone();
        let track_ext = track.ext_hint.clone();
        let track_meta = TrackMeta {
            duration_ms: track.duration_ms,
            sample_rate: track.sample_rate,
            album: track.album.clone(),
            artist: track.artist.clone(),
            format: Some(track.format.clone()),
        };
        let track_duration = track.duration_ms;

        let queue: Vec<PathBuf> = self
            .entries
            .iter()
            .filter_map(|item| match item {
                LibraryItem::Track(t) => Some(t.path.clone()),
                _ => None,
            })
            .collect();
        let queue_index = queue.iter().position(|p| p == &track_path);
        self.queued_next.clear();
        cmd_tx
            .send(Command::Play {
                path: track_path.clone(),
                ext_hint: track_ext,
            })
            .ok();
        self.ensure_meta_for_path(&track_path);
        self.now_playing_index = Some(index);
        self.now_playing_path = Some(track_path.clone());
        self.now_playing_meta = Some(track_meta);
        self.playing_queue = queue;
        self.playing_queue_index = queue_index;
        self.auto_advance_armed = true;
        self.remote_duration_ms = track_duration;
        self.remote_played_frames = Some(0);
        self.remote_paused = Some(false);
        self.mark_queue_dirty();
    }

    fn play_track_path(&mut self, path: PathBuf, cmd_tx: &Sender<Command>) {
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        self.queued_next.clear();
        cmd_tx
            .send(Command::Play {
                path: path.clone(),
                ext_hint,
            })
            .ok();

        self.ensure_meta_for_path(&path);

        let meta = self
            .meta_cache
            .get(&path)
            .cloned()
            .unwrap_or_default();

        self.now_playing_path = Some(path.clone());
        self.now_playing_index = self.entries.iter().position(|item| item.path() == path);
        self.now_playing_meta = Some(meta.clone());
        self.meta_cache.insert(path.clone(), meta.clone());
        if let Some(idx) = self.now_playing_index {
            self.select_index(idx);
        }
        if let Some(parent) = path.parent() {
            if parent == self.dir.as_path() {
                let queue: Vec<PathBuf> = self
                    .entries
                    .iter()
                    .filter_map(|item| match item {
                        LibraryItem::Track(t) => Some(t.path.clone()),
                        _ => None,
                    })
                    .collect();
                let queue_index = queue.iter().position(|p| p == &path);
                self.playing_queue = queue;
                self.playing_queue_index = queue_index;
            }
        }
        self.remote_duration_ms = meta.duration_ms;
        self.remote_played_frames = Some(0);
        self.remote_paused = Some(false);
        self.auto_advance_armed = true;
        self.mark_queue_dirty();
    }

    fn maybe_auto_advance(&mut self, cmd_tx: &Sender<Command>) {
        if !self.auto_advance_armed {
            return;
        }

        let (Some(sr), Some(dur_ms), Some(frames), Some(_paused)) = (
            self.remote_sample_rate,
            self.remote_duration_ms,
            self.remote_played_frames,
            self.remote_paused,
        ) else {
            return;
        };

        if sr == 0 || dur_ms == 0 {
            return;
        }
        let elapsed_ms = (frames as f64 * 1000.0) / (sr as f64);
        if elapsed_ms + 50.0 < dur_ms as f64 {
            return;
        }

        self.auto_advance_armed = false;
        if let Some(next_path) = self.queued_next.pop_front() {
            self.play_track_path(next_path, cmd_tx);
            self.status = "Auto next (queued)".into();
            return;
        }

        if let Some(idx) = self.playing_queue_index {
            if let Some(next_path) = self.playing_queue.get(idx + 1).cloned() {
                self.playing_queue_index = Some(idx + 1);
                self.play_track_path(next_path, cmd_tx);
                self.status = "Auto next".into();
                return;
            }
        }

        self.status = "End of list".into();
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
                Event::Progress { sent, total } => app.last_progress = Some((sent, total)),
                Event::RemoteTrackInfo { sample_rate, channels, duration_ms } => {
                    app.remote_sample_rate = Some(sample_rate);
                    app.remote_channels = Some(channels);
                    if app.remote_duration_ms.is_none() && duration_ms.is_some() {
                        app.remote_duration_ms = duration_ms;
                    }
                }
                Event::RemotePlaybackPos { played_frames, paused } => {
                    app.remote_played_frames = Some(played_frames);
                    app.remote_paused = Some(paused);
                    app.maybe_auto_advance(&cmd_tx);
                }
                Event::Error(e) => app.status = format!("Error: {e}"),
            }
        }

        app.drain_meta_results();
        app.drain_scan_results();

        terminal.draw(|f| render::draw(f, app))?;

        let timeout = tick.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).context("poll terminal events")? {
            if let CEvent::Key(k) = event::read().context("read terminal event")? {
                match k.code {
                    KeyCode::Char('q') => {
                        cmd_tx.send(Command::Quit).ok();
                        return Ok(());
                    }
                    KeyCode::Up => app.select_prev(),
                    KeyCode::Down => app.select_next(),
                    KeyCode::Left | KeyCode::Backspace => {
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
                    KeyCode::Char('n') => {
                        // Immediate skip: tell worker "Next", then start the next selected track.
                        cmd_tx.send(Command::Next).ok();
                        if let Some(next_path) = app.queued_next.pop_front() {
                            app.play_track_path(next_path, &cmd_tx);
                            app.status = "Skipping (queued)".into();
                            continue;
                        }
                        if let Some(idx) = app.playing_queue_index {
                            if let Some(next_path) = app.playing_queue.get(idx + 1).cloned() {
                                app.playing_queue_index = Some(idx + 1);
                                app.play_track_path(next_path, &cmd_tx);
                                app.status = "Skipping (next)".into();
                                continue;
                            }
                        }

                        let start = app.selected_index().unwrap_or(0);
                        if let Some(next_index) = app.next_track_index_from(start) {
                            app.select_index(next_index);
                            app.play_track_at(next_index, &cmd_tx);
                        } else {
                            app.status = "End of list".into();
                        }
                    }
                    KeyCode::Char('k') => {
                        app.toggle_queue_selected();
                    }
                    KeyCode::Char('p') => {
                        app.jump_to_playing()?;
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();
        }
    }
}

fn list_entries_with_parent(dir: &PathBuf) -> Result<Vec<LibraryItem>> {
    let mut entries = library::list_entries(dir)?;
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

    fn track_item(path: &str, artist: &str) -> LibraryItem {
        LibraryItem::Track(Track {
            path: PathBuf::from(path),
            file_name: path
                .rsplit('/')
                .next()
                .unwrap_or("file.flac")
                .to_string(),
            ext_hint: "flac".to_string(),
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
        let (meta_tx, _meta_req_rx) = unbounded::<MetaReq>();
        let (_meta_done_tx, meta_rx) = unbounded::<MetaResp>();
        App::new(
            "127.0.0.1:5555".parse().unwrap(),
            PathBuf::from("/music"),
            entries,
            scan_tx,
            scan_rx,
            meta_tx,
            meta_rx,
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
    fn queueing_track_requests_meta() {
        let entries = vec![track_item("/music/a.flac", "A")];
        let mut app = app_with_entries(entries);
        app.list_state.select(Some(0));
        app.toggle_queue_selected();
        // ensure_meta_for_path will early-return if meta_cache already contains the path
        assert!(app.meta_cache.contains_key(&PathBuf::from("/music/a.flac")));
    }

    #[test]
    fn play_track_clears_queue() {
        let entries = vec![
            track_item("/music/a.flac", "A"),
            track_item("/music/b.flac", "B"),
        ];
        let mut app = app_with_entries(entries);
        app.queued_next.push_back(PathBuf::from("/music/a.flac"));
        app.list_state.select(Some(1));
        let (cmd_tx, _cmd_rx) = unbounded::<Command>();
        app.play_track_at(1, &cmd_tx);
        assert!(app.queued_next.is_empty());
    }
}
