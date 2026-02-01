//! Ratatui UI loop.
//!
//! Keys:
//! - Up/Down: move selection
//! - Left/Backspace: go to parent dir
//! - Enter: play selected (or enter dir)
//! - Space: pause/resume
//! - n: next (immediate skip)
//! - r: rescan directory
//! - q: quit

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::collections::{HashMap, VecDeque};
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
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Terminal,
};

use crate::library::{self, LibraryItem, Track, TrackMeta};
use crate::worker::{self, Command, Event};

pub fn run_tui(addr: SocketAddr, dir: PathBuf) -> Result<()> {
    let entries = list_entries_with_parent(&dir)?;
    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    let (evt_tx, evt_rx) = unbounded::<Event>();

    std::thread::spawn({
        let addr = addr;
        move || worker::worker_main(addr, cmd_rx, evt_tx)
    });

    let mut app = App::new(addr, dir, entries);

    let mut term = init_terminal()?;
    let result = ui_loop(&mut term, &mut app, cmd_tx, evt_rx);

    restore_terminal(&mut term)?;
    result
}

struct App {
    addr: SocketAddr,
    dir: PathBuf,
    entries: Vec<LibraryItem>,
    list_state: ListState,
    now_playing_index: Option<usize>,
    now_playing_path: Option<PathBuf>,
    now_playing_meta: Option<TrackMeta>,
    playing_queue: Vec<PathBuf>,
    playing_queue_index: Option<usize>,
    queued_next: VecDeque<PathBuf>,
    meta_cache: HashMap<PathBuf, TrackMeta>,
    auto_advance_armed: bool,

    status: String,
    last_progress: Option<(u64, Option<u64>)>, // sent, total

    remote_sample_rate: Option<u32>,
    remote_channels: Option<u16>,
    remote_duration_ms: Option<u64>,
    remote_played_frames: Option<u64>,
    remote_paused: Option<bool>,
}

impl App {
    fn new(addr: SocketAddr, dir: PathBuf, entries: Vec<LibraryItem>) -> Self {
        let mut list_state = ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
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
            meta_cache: HashMap::new(),
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
        self.entries = list_entries_with_parent(&self.dir)?;
        self.meta_cache.clear();
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
            .and_then(|path| self.entries.iter().position(|item| item.path() == path));
        if self.entries.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        Ok(())
    }

    fn go_parent(&mut self) -> Result<()> {
        if let Some(parent) = self.dir.parent() {
            self.dir = parent.to_path_buf();
            self.rescan()?;
            self.status = format!("Entered {:?}", self.dir);
        }
        Ok(())
    }

    fn enter_dir(&mut self, dir: PathBuf) -> Result<()> {
        self.dir = dir;
        self.rescan()?;
        self.status = format!("Entered {:?}", self.dir);
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
        self.rescan()?;
        if let Some(idx) = self.entries.iter().position(|item| item.path() == path) {
            self.now_playing_index = Some(idx);
            self.select_index(idx);
            self.status = "Jumped to playing".into();
        } else {
            self.status = "Playing track not in folder".into();
        }
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

        if let Some(pos) = self.queued_next.iter().position(|p| p == &track.path) {
            self.queued_next.remove(pos);
            self.status = "Unqueued".into();
        } else {
            self.queued_next.push_back(track.path.clone());
            self.status = "Queued".into();
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
        let queue: Vec<PathBuf> = self
            .entries
            .iter()
            .filter_map(|item| match item {
                LibraryItem::Track(t) => Some(t.path.clone()),
                _ => None,
            })
            .collect();
        let queue_index = queue.iter().position(|p| p == &track.path);
        cmd_tx
            .send(Command::Play {
                path: track.path.clone(),
                ext_hint: track.ext_hint.clone(),
            })
            .ok();
        self.now_playing_index = Some(index);
        self.now_playing_path = Some(track.path.clone());
        self.now_playing_meta = Some(TrackMeta {
            duration_ms: track.duration_ms,
            sample_rate: track.sample_rate,
            album: track.album.clone(),
            artist: track.artist.clone(),
            format: Some(track.format.clone()),
        });
        self.playing_queue = queue;
        self.playing_queue_index = queue_index;
        self.auto_advance_armed = true;
        self.remote_duration_ms = track.duration_ms;
        self.remote_played_frames = Some(0);
        self.remote_paused = Some(false);
    }

    fn play_track_path(&mut self, path: PathBuf, cmd_tx: &Sender<Command>) {
        let ext_hint = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let meta = library::probe_track_meta(&path, &ext_hint);
        cmd_tx
            .send(Command::Play {
                path: path.clone(),
                ext_hint,
            })
            .ok();

        self.now_playing_path = Some(path.clone());
        self.now_playing_index = self.entries.iter().position(|item| item.path() == path);
        self.now_playing_meta = Some(meta.clone());
        self.meta_cache.insert(path.clone(), meta.clone());
        if let Some(idx) = self.now_playing_index {
            self.select_index(idx);
        }
        if let Some(parent) = path.parent() {
            if let Ok(entries) = library::list_entries(parent) {
                let queue: Vec<PathBuf> = entries
                    .into_iter()
                    .filter_map(|item| match item {
                        LibraryItem::Track(t) => Some(t.path),
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

        terminal.draw(|f| draw(f, app))?;

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
                        app.status = "Rescanned".into();
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

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let remote_status = match (
        app.remote_played_frames,
        app.remote_sample_rate,
        app.remote_duration_ms,
        app.remote_paused,
    ) {
        (Some(fr), Some(sr), dur, Some(p)) if sr > 0 => {
            let elapsed_ms = (fr as f64 * 1000.0) / (sr as f64);
            let state = if p { "paused" } else { "playing" };
            match dur {
                Some(total_ms) if total_ms > 0 => {
                    let elapsed = format_duration_ms(elapsed_ms as u64);
                    let total = format_duration_ms(total_ms);
                    let remaining = format_duration_ms(total_ms.saturating_sub(elapsed_ms as u64));
                    let pct = (elapsed_ms / total_ms as f64 * 100.0).clamp(0.0, 100.0);
                    format!(
                        "remote: {elapsed} / {total} (left {remaining}, {pct:.1}%) [{state}]"
                    )
                }
                _ => format!("remote: {:.1}s [{state}]", elapsed_ms / 1000.0),
            }
        }
        _ => "remote: -".to_string(),
    };

    let remote_gauge = match (
        app.remote_played_frames,
        app.remote_sample_rate,
        app.remote_duration_ms,
        app.remote_paused,
    ) {
        (Some(fr), Some(sr), Some(total_ms), Some(p)) if sr > 0 && total_ms > 0 => {
            let elapsed_ms = (fr as f64 * 1000.0) / (sr as f64);
            let ratio = (elapsed_ms / total_ms as f64).clamp(0.0, 1.0);
            let state = if p { "paused" } else { "playing" };
            let elapsed = format_duration_ms(elapsed_ms as u64);
            let remaining = format_duration_ms(total_ms.saturating_sub(elapsed_ms as u64));
            Some((
                ratio,
                format!("{elapsed} elapsed • {remaining} left [{state}]"),
            ))
        }
        _ => None,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(5), Constraint::Length(7)])
        .split(f.area());

    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Length(8)])
        .split(chunks[0]);

    let header = Paragraph::new(vec![
        Line::from(format!("audio-send  →  {}", app.addr)),
        Line::from(format!("dir: {:?}", app.dir)),
    ])
        .block(Block::default().borders(Borders::ALL).title("Target"));
    f.render_widget(header, top_chunks[0]);

    let playing_name = app
        .now_playing_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("-");
    let playing_path = app
        .now_playing_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "-".into());
    let playing_album = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.album.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("-");
    let playing_artist = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.artist.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("-");
    let playing_format = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.format.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("-");
    let source_sr = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.sample_rate);
    let output_sr = app.remote_sample_rate;
    let sr_line = match (source_sr, output_sr) {
        (Some(src), Some(out)) => format!("sample rate: {src} Hz → {out} Hz"),
        (Some(src), None) => format!("sample rate: {src} Hz → -"),
        (None, Some(out)) => format!("sample rate: - → {out} Hz"),
        (None, None) => "sample rate: -".into(),
    };

    let now_playing = Paragraph::new(vec![
        Line::from(format!("file: {playing_name}")),
        Line::from(format!("path: {playing_path}")),
        Line::from(format!("album: {playing_album}")),
        Line::from(format!("artist: {playing_artist}")),
        Line::from(format!("format: {playing_format}")),
        Line::from(sr_line),
    ])
    .block(Block::default().borders(Borders::ALL).title("Now Playing"));
    f.render_widget(now_playing, top_chunks[1]);

    let max_name_len = app
        .entries
        .iter()
        .map(|item| item.name().len() + if item.is_dir() { 1 } else { 0 })
        .max()
        .unwrap_or(0);

    let items: Vec<ListItem> = app
        .entries
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let name = if item.is_dir() {
                format!("{}/", item.name())
            } else {
                item.name().to_string()
            };
            let label = match item.duration_ms() {
                Some(ms) => {
                    let dur = format_duration_ms(ms);
                    format!("{:<width$}  [{dur}]", name, width = max_name_len)
                }
                None if item.is_dir() => format!("{:<width$}  [dir]", name, width = max_name_len),
                None => name,
            };
            let mut label = label;
            if app.now_playing_index == Some(idx) {
                label = format!("{label}  [playing]");
            }
            if app
                .entries
                .get(idx)
                .and_then(|entry| match entry {
                    LibraryItem::Track(t) => Some(&t.path),
                    _ => None,
                })
                .map(|path| app.queued_next.iter().any(|p| p == path))
                .unwrap_or(false)
            {
                label = format!("{label}  [queued]");
            }
            ListItem::new(label)
        })
        .collect();

    let mid_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Entries"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, mid_chunks[0], &mut app.list_state);

    let queue_width = mid_chunks[1].width as usize;
    let queued_items: Vec<ListItem> = if app.queued_next.is_empty() {
        vec![ListItem::new("<empty>")]
    } else {
        app.queued_next
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let meta = app
                    .meta_cache
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| {
                        let ext_hint = path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        library::probe_track_meta(path, &ext_hint)
                    });
                let artist = meta.artist.unwrap_or_else(|| "-".into());
                let album = meta.album.unwrap_or_else(|| "-".into());
                let song = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<file>");
                let label = format!("{:>2}. {} - {} - {}", i + 1, artist, album, song);
                ListItem::new(truncate_label(&label, queue_width.saturating_sub(1)))
            })
            .collect()
    };

    let queue_list = List::new(queued_items)
        .block(Block::default().borders(Borders::ALL).title("Queue"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(queue_list, mid_chunks[1]);

    let progress_line = match app.last_progress {
        Some((sent, Some(total))) if total > 0 => {
            format!("sent: {} / {} bytes ({:.1}%)", sent, total, (sent as f64 * 100.0) / total as f64)
        }
        Some((sent, None)) => format!("sent: {} bytes", sent),
        _ => "sent: -".to_string(),
    };

    let footer_block = Block::default().borders(Borders::ALL).title("Status");
    let footer_inner = footer_block.inner(chunks[2]);
    f.render_widget(footer_block, chunks[2]);

    let footer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(footer_inner);

    f.render_widget(
        Paragraph::new(Line::from(format!("status: {}", app.status))),
        footer_chunks[0],
    );
    f.render_widget(Paragraph::new(Line::from(progress_line)), footer_chunks[1]);
    f.render_widget(Paragraph::new(Line::from(remote_status)), footer_chunks[2]);
    if let Some((ratio, label)) = remote_gauge {
        let gauge_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(10), Constraint::Length(40)])
            .split(footer_chunks[3]);

        let gauge = Gauge::default()
            .ratio(ratio)
            .style(Style::default().fg(Color::Black).bg(Color::White))
            .gauge_style(Style::default().fg(Color::White).bg(Color::Black));
        f.render_widget(gauge, gauge_chunks[0]);
        f.render_widget(
            Paragraph::new(Line::from(label)).alignment(Alignment::Right),
            gauge_chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from("remote progress: -")),
            footer_chunks[3],
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(
            "keys: ↑/↓ select | Enter play/enter | ←/Backspace parent | Space pause | n next | k queue | p playing | r rescan | q quit",
        )),
        footer_chunks[4],
    );
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

fn truncate_label(label: &str, max: usize) -> String {
    if max == 0 || label.len() <= max {
        return label.to_string();
    }
    if max <= 3 {
        return label[..max].to_string();
    }
    let cut = max - 3;
    format!("{}...", &label[..cut])
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}
