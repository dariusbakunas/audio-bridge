use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
};

use crate::library::{self, LibraryItem};

use super::app::App;

pub(crate) fn draw(f: &mut ratatui::Frame, app: &mut App) {
    app.refresh_auto_preview_if_needed();

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
    let mut upcoming: Vec<(PathBuf, bool)> = Vec::new();
    let mut queued_set: HashSet<PathBuf> = HashSet::new();
    for path in app.queued_next.iter() {
        queued_set.insert(path.clone());
        upcoming.push((path.clone(), true));
    }
    for path in app.auto_preview.iter() {
        if queued_set.contains(path) {
            continue;
        }
        upcoming.push((path.clone(), false));
    }

    let queued_items: Vec<ListItem> = if upcoming.is_empty() {
        vec![ListItem::new("<empty>")]
    } else {
        upcoming
            .iter()
            .enumerate()
            .map(|(i, (path, is_queued))| {
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
                let tag = if *is_queued { "queued" } else { "auto" };
                let label = format!("{:>2}. {} - {} - {} [{tag}]", i + 1, artist, album, song);
                ListItem::new(truncate_label(&label, queue_width.saturating_sub(1)))
            })
            .collect()
    };

    let queue_list = List::new(queued_items)
        .block(Block::default().borders(Borders::ALL).title("Up Next"))
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

fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}
