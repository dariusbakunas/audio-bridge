//! Ratatui UI loop.
//!
//! Keys:
//! - Up/Down: move selection
//! - Enter: play selected
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
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};

use crate::library::{self, Track};
use crate::worker::{self, Command, Event};

pub fn run_tui(addr: SocketAddr, dir: PathBuf) -> Result<()> {
    let tracks = library::list_tracks(&dir)?;
    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    let (evt_tx, evt_rx) = unbounded::<Event>();

    std::thread::spawn({
        let addr = addr;
        move || worker::worker_main(addr, cmd_rx, evt_tx)
    });

    let mut app = App::new(addr, dir, tracks);

    let mut term = init_terminal()?;
    let result = ui_loop(&mut term, &mut app, cmd_tx, evt_rx);

    restore_terminal(&mut term)?;
    result
}

struct App {
    addr: SocketAddr,
    dir: PathBuf,
    tracks: Vec<Track>,
    list_state: ListState,

    status: String,
    last_progress: Option<(u64, Option<u64>)>, // sent, total
}

impl App {
    fn new(addr: SocketAddr, dir: PathBuf, tracks: Vec<Track>) -> Self {
        let mut list_state = ListState::default();
        if !tracks.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            addr,
            dir,
            tracks,
            list_state,
            status: "Ready".into(),
            last_progress: None,
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    fn selected_track(&self) -> Option<&Track> {
        self.selected_index().and_then(|i| self.tracks.get(i))
    }

    fn select_next(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = (i + 1).min(self.tracks.len() - 1);
        self.list_state.select(Some(ni));
    }

    fn select_prev(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        let i = self.selected_index().unwrap_or(0);
        let ni = i.saturating_sub(1);
        self.list_state.select(Some(ni));
    }

    fn rescan(&mut self) -> Result<()> {
        self.tracks = library::list_tracks(&self.dir)?;
        if self.tracks.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
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
                    KeyCode::Char('r') => {
                        app.rescan()?;
                        app.status = "Rescanned".into();
                    }
                    KeyCode::Enter => {
                        if let Some(t) = app.selected_track() {
                            cmd_tx
                                .send(Command::Play {
                                    path: t.path.clone(),
                                    ext_hint: t.ext_hint.clone(),
                                })
                                .ok();
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(3)])
        .split(f.area());

    let header = Paragraph::new(vec![
        Line::from(format!("audio-send  →  {}", app.addr)),
        Line::from(format!("dir: {:?}", app.dir)),
    ])
        .block(Block::default().borders(Borders::ALL).title("Target"));
    f.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .tracks
        .iter()
        .map(|t| ListItem::new(t.file_name.clone()))
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Files (.flac/.wav)"))
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

    let footer = Paragraph::new(vec![
        Line::from(format!("status: {}", app.status)),
        Line::from(progress_line),
        Line::from("keys: ↑/↓ select | Enter play | Space pause | n next | r rescan | q quit"),
    ])
        .block(Block::default().borders(Borders::ALL).title("Status"));

    f.render_widget(footer, chunks[2]);
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