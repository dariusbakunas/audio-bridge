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

use crate::library::{self, LibraryItem, Track};
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
                        } else if let Some(t) = app.selected_track() {
                            cmd_tx
                                .send(Command::Play {
                                    path: t.path.clone(),
                                    ext_hint: t.ext_hint.clone(),
                                })
                                .ok();
                            app.remote_duration_ms = t.duration_ms;
                            app.remote_played_frames = Some(0);
                            app.remote_paused = Some(false);
                        }
                    }
                    KeyCode::Char(' ') => {
                        cmd_tx.send(Command::PauseToggle).ok();
                    }
                    KeyCode::Char('n') => {
                        // Immediate skip: tell worker "Next", then start the next selected track.
                        cmd_tx.send(Command::Next).ok();
                        app.select_next();
                        if let Some(t) = app.selected_track() {
                            cmd_tx
                                .send(Command::Play {
                                    path: t.path.clone(),
                                    ext_hint: t.ext_hint.clone(),
                                })
                                .ok();
                            app.remote_duration_ms = t.duration_ms;
                            app.remote_played_frames = Some(0);
                            app.remote_paused = Some(false);
                        }
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
        .constraints([Constraint::Length(4), Constraint::Min(5), Constraint::Length(7)])
        .split(f.area());

    let header = Paragraph::new(vec![
        Line::from(format!("audio-send  →  {}", app.addr)),
        Line::from(format!("dir: {:?}", app.dir)),
    ])
        .block(Block::default().borders(Borders::ALL).title("Target"));
    f.render_widget(header, chunks[0]);

    let max_name_len = app
        .entries
        .iter()
        .map(|item| item.name().len() + if item.is_dir() { 1 } else { 0 })
        .max()
        .unwrap_or(0);

    let items: Vec<ListItem> = app
        .entries
        .iter()
        .map(|item| {
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
            ListItem::new(label)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Entries"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, chunks[1], &mut app.list_state);

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
            .constraints([Constraint::Min(10), Constraint::Length(30)])
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
            "keys: ↑/↓ select | Enter play/enter | ←/Backspace parent | Space pause | n next | r rescan | q quit",
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
