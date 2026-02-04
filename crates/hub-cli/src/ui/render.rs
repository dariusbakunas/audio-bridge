use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph},
};

use crate::library::LibraryItem;

use super::app::App;

pub(crate) fn draw(f: &mut ratatui::Frame, app: &mut App) {
    app.refresh_auto_preview_if_needed();

    let remote_status = match (app.remote_elapsed_ms, app.remote_duration_ms, app.remote_paused) {
        (Some(elapsed_ms), dur, Some(p)) => {
            let state = if p { "paused" } else { "playing" };
            match dur {
                Some(total_ms) if total_ms > 0 => {
                    let elapsed = format_duration_ms(elapsed_ms);
                    let total = format_duration_ms(total_ms);
                    let remaining = format_duration_ms(total_ms.saturating_sub(elapsed_ms));
                    let pct = (elapsed_ms as f64 / total_ms as f64 * 100.0).clamp(0.0, 100.0);
                    let mut base = format!(
                        "remote: {elapsed} / {total} (left {remaining}, {pct:.1}%) [{state}]"
                    );
                    let extra = signal_path_line(app);
                    if !extra.is_empty() {
                        base.push_str(" | ");
                        base.push_str(&extra);
                    }
                    base
                }
                _ => {
                    let mut base = format!("remote: {:.1}s [{state}]", elapsed_ms as f64 / 1000.0);
                    let extra = signal_path_line(app);
                    if !extra.is_empty() {
                        base.push_str(" | ");
                        base.push_str(&extra);
                    }
                    base
                }
            }
        }
        _ => {
            let extra = signal_path_line(app);
            if extra.is_empty() {
                "remote: -".to_string()
            } else {
                format!("remote: - | {extra}")
            }
        }
    };

    let remote_gauge = match (app.remote_elapsed_ms, app.remote_duration_ms, app.remote_paused) {
        (Some(elapsed_ms), Some(total_ms), Some(p)) if total_ms > 0 => {
            let ratio = (elapsed_ms as f64 / total_ms as f64).clamp(0.0, 1.0);
            let state = if p { "paused" } else { "playing" };
            let elapsed = format_duration_ms(elapsed_ms);
            let remaining = format_duration_ms(total_ms.saturating_sub(elapsed_ms));
            Some((
                ratio,
                format!("{elapsed} elapsed • {remaining} left [{state}]"),
            ))
        }
        _ => None,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(16), Constraint::Min(5), Constraint::Length(7)])
        .split(f.area());

    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(10)])
        .split(chunks[0]);

    let output_line = app
        .remote_output_id
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "-".into());
    let header = Paragraph::new(vec![
        Line::from(format!("hub-cli  →  {}", app.server)),
        Line::from(format!("dir: {:?}", app.dir)),
        Line::from(format!("output: {output_line}")),
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
    let playing_channels = app
        .remote_channels
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| "-".into());
    let source_sr = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.sample_rate);
    let output_sr = app.remote_output_sample_rate;
    let sr_line = match (source_sr, output_sr) {
        (Some(src), Some(out)) => format!("sample rate: {src} Hz -> {out} Hz"),
        (Some(src), None) => format!("sample rate: {src} Hz -> -"),
        (None, Some(out)) => format!("sample rate: - -> {out} Hz"),
        (None, None) => "sample rate: -".into(),
    };

    let now_playing = Paragraph::new(vec![
        Line::from(format!("file: {playing_name}")),
        Line::from(format!("path: {playing_path}")),
        Line::from(format!("album: {playing_album}")),
        Line::from(format!("artist: {playing_artist}")),
        Line::from(format!("format: {playing_format}")),
        Line::from(format!("channels: {playing_channels}")),
        Line::from(sr_line),
    ])
    .block(Block::default().borders(Borders::ALL).title("Now Playing"));
    f.render_widget(now_playing, top_chunks[1]);

    let queue_after_index: Vec<PathBuf> = app.queued_next.iter().cloned().collect();
    let queue_after_set: HashSet<PathBuf> = queue_after_index.iter().cloned().collect();

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
                .map(|path| app.now_playing_path.as_ref() != Some(path) && queue_after_set.contains(path))
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

    let list_block = Block::default().borders(Borders::ALL).title("Entries");
    app.list_view_height = list_block.inner(mid_chunks[0]).height as usize;
    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, mid_chunks[0], &mut app.list_state);

    let queue_width = mid_chunks[1].width as usize;
    let mut upcoming: Vec<(PathBuf, bool)> = Vec::new();
    let mut queued_set: HashSet<PathBuf> = HashSet::new();
    for path in queue_after_index.iter().cloned() {
        if app.now_playing_path.as_ref() == Some(&path) {
            continue;
        }
        queued_set.insert(path.clone());
        upcoming.push((path.clone(), true));
    }
    // For now, "Up Next" is strictly the server queue.

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
                    .unwrap_or_default();
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
        ])
        .split(footer_inner);

    f.render_widget(
        Paragraph::new(Line::from(format!("status: {}", app.status))),
        footer_chunks[0],
    );
    f.render_widget(Paragraph::new(Line::from(remote_status)), footer_chunks[1]);
    if let Some((ratio, label)) = remote_gauge {
        let gauge_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(10), Constraint::Length(40)])
            .split(footer_chunks[2]);

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
            footer_chunks[2],
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(
            "keys: ↑/↓ select | Enter play/enter | Space pause | n next | ←/→ seek | o outputs | l logs | h help | q quit",
        )),
        footer_chunks[3],
    );

    if app.help_open {
        let area = centered_rect(70, 70, f.area());
        f.render_widget(Clear, area);
        let help = [
            "Navigation",
            "  ↑/↓          select",
            "  PgUp/PgDn    page",
            "  Enter        play/enter dir",
            "  Backspace    parent dir",
            "",
            "Playback",
            "  Space        pause/resume",
            "  n            next",
            "  ←/→          seek −5s / +5s",
            "  Shift+←/→    seek −30s / +30s",
            "",
            "Queue",
            "  k            queue selected track",
            "  K            queue all tracks in dir",
            "",
            "Other",
            "  p            jump to playing",
            "  r            rescan",
            "  o            outputs",
            "  l            logs",
            "  h or ?       help",
            "  q            quit",
            "  Esc          close modal",
        ]
        .join("\n");
        let block = Block::default().title("Help").borders(Borders::ALL);
        f.render_widget(Paragraph::new(help).block(block), area);
    }

    if app.outputs_open {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);
        let mut items = Vec::new();
        if let Some(err) = app.outputs_error.as_ref() {
            items.push(ListItem::new(format!("error: {err}")));
        }
        if app.outputs.is_empty() {
            items.push(ListItem::new("<no outputs>"));
        } else {
            for out in &app.outputs {
                let active = app
                    .outputs_active_id
                    .as_ref()
                    .map(|id| id == &out.id)
                    .unwrap_or(false);
                let tag = if active { " *" } else { "" };
                let bridge = out
                    .bridge_name
                    .as_deref()
                    .or(out.bridge_id.as_deref())
                    .unwrap_or("-");
                let rates = out
                    .supported_rates
                    .map(|(min_hz, max_hz)| format!("{min_hz}-{max_hz} Hz"))
                    .unwrap_or_else(|| "-".to_string());
                let label = format!("{}  [{}]  {}{}", out.name, bridge, rates, tag);
                items.push(ListItem::new(label));
            }
        }
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Select Output (Enter to apply, Esc to close)"))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, area, &mut app.outputs_state);
    }

    if app.logs_open {
        let area = centered_rect(90, 80, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Logs (Esc to close, ↑/↓ scroll)");
        let inner = block.inner(area);
        let height = inner.height as usize;
        let total = app.logs.len();
        let end = total.saturating_sub(app.logs_scroll);
        let start = end.saturating_sub(height);
        let mut items = Vec::new();
        for line in app.logs.iter().skip(start).take(end.saturating_sub(start)) {
            items.push(ListItem::new(line.clone()));
        }
        if items.is_empty() {
            items.push(ListItem::new("<no logs>"));
        }
        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

fn signal_path_line(app: &App) -> String {
    let mut parts = Vec::new();
    if let Some(codec) = app.remote_source_codec.as_deref() {
        let mut src = codec.to_string();
        if let Some(bit) = app.remote_source_bit_depth {
            src.push(' ');
            src.push_str(&format!("{bit}b"));
        }
        if let Some(container) = app.remote_container.as_deref() {
            src.push(' ');
            src.push_str(container);
        }
        parts.push(format!("src {src}"));
    }
    if let Some(kbps) = app.remote_bitrate_kbps {
        parts.push(format!("br {kbps} kbps"));
    }
    if let Some(out_sr) = app.remote_output_sample_rate {
        let fmt = app
            .remote_output_sample_format
            .as_deref()
            .unwrap_or("-");
        parts.push(format!("out {out_sr}Hz {fmt}"));
    }
    if let Some(resampling) = app.remote_resampling {
        if resampling {
            if let (Some(from), Some(to)) = (app.remote_resample_from_hz, app.remote_resample_to_hz) {
                parts.push(format!("rs {from}->{to}"));
            } else {
                parts.push("rs yes".to_string());
            }
        } else {
            parts.push("rs no".to_string());
        }
    }
    parts.join(" | ")
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

fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    horizontal[1]
}
