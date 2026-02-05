//! UI view-models for the TUI.
//!
//! This module converts `App` state into render-ready strings, labels,
//! and modal payloads so `render.rs` stays layout-focused.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::library::LibraryItem;
use crate::ui::app::App;

pub(crate) struct UiView {
    pub(crate) remote_status: String,
    pub(crate) remote_gauge: Option<(f64, String)>,
    pub(crate) buffer_ratio: Option<f64>,
    pub(crate) header_lines: Vec<String>,
    pub(crate) now_playing_summary: Vec<String>,
    pub(crate) now_playing_panel: Option<NowPlayingPanel>,
    pub(crate) entry_labels: Vec<String>,
    pub(crate) queue_labels: Vec<String>,
    pub(crate) status_line: String,
    pub(crate) keys_line: String,
    pub(crate) active_modal: Option<UiModal>,
}

pub(crate) enum UiModal {
    Help { title: String, body: String, layout: ModalLayout },
    Outputs { title: String, items: Vec<String>, empty: bool, error: Option<String>, layout: ModalLayout },
    Logs { title: String, empty: bool, layout: ModalLayout },
    ClearQueue { title: String, body: String, layout: ModalLayout },
}

pub(crate) struct ModalLayout {
    pub(crate) width_pct: u16,
    pub(crate) height_pct: u16,
}

pub(crate) struct NowPlayingPanel {
    pub(crate) title: String,
    pub(crate) body: String,
}

impl UiView {
    pub(crate) fn from_app(app: &App) -> Self {
        let remote_status = build_remote_status(app);
        let (remote_gauge, buffer_ratio) = build_remote_gauge(app);
        let output_line = app
            .remote_output_id
            .as_deref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".into());
        let header_lines = vec![
            format!("hub-cli  →  {}", app.server),
            format!("dir: {:?}", app.dir),
            format!("output: {output_line}"),
        ];
        let now_playing_summary = build_now_playing_summary(app);
        let now_playing_panel = if app.now_playing_open {
            Some(build_now_playing_panel(app))
        } else {
            None
        };
        let queue_labels = build_queue_labels(app);
        let entry_labels = build_entry_labels(app);
        let outputs_labels = build_outputs_labels(app);
        let outputs_empty = app.outputs.is_empty();
        let outputs_error = app.outputs_error.clone();
        let status_line = format!("status: {}", app.status);
        let keys_line = "keys: ↑/↓ select | Enter play/enter | Space pause | s stop | n next | c clear | ←/→ seek | o outputs | l logs | i info | h help | q quit".to_string();
        let help_lines = build_help_lines();
        let (logs_title, logs_empty) = build_logs_view(app);
        let active_modal = build_active_modal(
            app,
            &help_lines,
            "Select Output (Enter to apply, Esc to close)",
            &outputs_labels,
            outputs_empty,
            outputs_error.clone(),
            &logs_title,
            logs_empty,
        );

        Self {
            remote_status,
            remote_gauge,
            buffer_ratio,
            header_lines,
            now_playing_summary,
            now_playing_panel,
            entry_labels,
            queue_labels,
            status_line,
            keys_line,
            active_modal,
        }
    }
}

fn build_remote_status(app: &App) -> String {
    match (app.remote_elapsed_ms, app.remote_duration_ms, app.remote_paused) {
        (Some(elapsed_ms), dur, Some(p)) => {
            let elapsed = format_duration_ms(elapsed_ms);
            let state = if p { "paused" } else { "playing" };
            match dur {
                Some(total) if total > 0 => {
                    let total = format_duration_ms(total);
                    let extra = signal_path_line(app);
                    if extra.is_empty() {
                        format!("remote: {elapsed}/{total} [{state}]")
                    } else {
                        format!("remote: {elapsed}/{total} [{state}] | {extra}")
                    }
                }
                _ => {
                    let extra = signal_path_line(app);
                    if extra.is_empty() {
                        format!("remote: {elapsed} [{state}]")
                    } else {
                        format!("remote: {elapsed} [{state}] | {extra}")
                    }
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
    }
}

fn build_remote_gauge(app: &App) -> (Option<(f64, String)>, Option<f64>) {
    let buffer_info = match (
        app.remote_buffered_frames,
        app.remote_buffer_capacity_frames,
        app.remote_output_sample_rate,
    ) {
        (Some(buffered), Some(capacity), Some(rate)) if capacity > 0 && rate > 0 => {
            let buffer_secs = buffered as f64 / rate as f64;
            let capacity_secs = capacity as f64 / rate as f64;
            if capacity_secs > 0.0 {
                let ratio = (buffer_secs / capacity_secs).clamp(0.0, 1.0);
                Some((ratio, format!("{buffer_secs:.1}s")))
            } else {
                None
            }
        }
        _ => None,
    };

    let remote_gauge = match (app.remote_elapsed_ms, app.remote_duration_ms, app.remote_paused) {
        (Some(elapsed_ms), Some(total_ms), Some(p)) if total_ms > 0 => {
            let ratio = (elapsed_ms as f64 / total_ms as f64).clamp(0.0, 1.0);
            let state = if p { "paused" } else { "playing" };
            let elapsed = format_duration_ms(elapsed_ms);
            let remaining = format_duration_ms(total_ms.saturating_sub(elapsed_ms));
            let buffer_label = buffer_info
                .as_ref()
                .map(|(_, secs)| format!(" • buf {secs}"))
                .unwrap_or_default();
            Some((
                ratio,
                format!(" {elapsed} elapsed • {remaining} left [{state}]{buffer_label}"),
            ))
        }
        _ => None,
    };

    (remote_gauge, buffer_info.map(|(ratio, _)| ratio))
}

fn build_now_playing_panel(app: &App) -> NowPlayingPanel {
    let title = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.format.clone())
        .unwrap_or_else(|| "Now Playing".to_string());
    let track = app
        .now_playing_path
        .as_ref()
        .and_then(|p| p.file_name().and_then(|s| s.to_str()))
        .unwrap_or("-");
    let artist = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.artist.clone())
        .unwrap_or_else(|| "-".into());
    let album = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.album.clone())
        .unwrap_or_else(|| "-".into());
    let duration = app
        .remote_duration_ms
        .map(format_duration_ms)
        .unwrap_or_else(|| "-".into());
    let elapsed = app
        .remote_elapsed_ms
        .map(format_duration_ms)
        .unwrap_or_else(|| "-".into());
    let source_codec = app.remote_source_codec.as_deref().unwrap_or("-");
    let source_bits = app
        .remote_source_bit_depth
        .map(|b| b.to_string())
        .unwrap_or_else(|| "-".into());
    let container = app.remote_container.as_deref().unwrap_or("-");
    let bitrate = app
        .remote_bitrate_kbps
        .map(|b| format!("{b} kbps"))
        .unwrap_or_else(|| "-".into());
    let out_sr = app
        .remote_output_sample_rate
        .map(|v| format!("{v} Hz"))
        .unwrap_or_else(|| "-".into());
    let out_fmt = app
        .remote_output_sample_format
        .as_deref()
        .unwrap_or("-");
    let out_dev = app
        .remote_output_id
        .as_deref()
        .unwrap_or("-");
    let resample = match app.remote_resampling {
        Some(true) => {
            if let (Some(from), Some(to)) = (app.remote_resample_from_hz, app.remote_resample_to_hz) {
                format!("{from} -> {to}")
            } else {
                "yes".to_string()
            }
        }
        Some(false) => "no".to_string(),
        None => "-".to_string(),
    };
    let body = [
        format!("Track: {track}"),
        format!("Artist: {artist}"),
        format!("Album: {album}"),
        format!("Position: {elapsed} / {duration}"),
        "".to_string(),
        format!("Source: {source_codec} {source_bits}b ({container})"),
        format!("Bitrate: {bitrate}"),
        "".to_string(),
        format!("Output: {out_sr} {out_fmt}"),
        format!("Resample: {resample}"),
        format!("Device: {out_dev}"),
        "".to_string(),
        "Press i or Esc to close".to_string(),
    ]
    .join("\n");

    NowPlayingPanel { title, body }
}

fn build_queue_labels(app: &App) -> Vec<String> {
    if app.queued_next.is_empty() {
        return Vec::new();
    }

    app.queued_next
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let meta = app.meta_cache.get(path).cloned().unwrap_or_default();
            let artist = meta.artist.unwrap_or_else(|| "-".into());
            let album = meta.album.unwrap_or_else(|| "-".into());
            let song = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("<file>");
            format!("{:>2}. {} - {} - {} [queued]", i + 1, artist, album, song)
        })
        .collect()
}

fn build_outputs_labels(app: &App) -> Vec<String> {
    app.outputs
        .iter()
        .map(|out| {
            let active = app
                .outputs_active_id
                .as_ref()
                .map(|id| id == &out.id)
                .unwrap_or(false);
            let tag = if active { " *" } else { "" };
            let bridge = out
                .provider_name
                .as_deref()
                .or(out.provider_id.as_deref())
                .unwrap_or("-");
            let rates = out
                .supported_rates
                .map(|(min_hz, max_hz)| format!("{min_hz}-{max_hz} Hz"))
                .unwrap_or_else(|| "-".to_string());
            format!("{}  [{}]  {}{}", out.name, bridge, rates, tag)
        })
        .collect()
}

fn build_entry_labels(app: &App) -> Vec<String> {
    let queue_after_set: HashSet<PathBuf> = app.queued_next.iter().cloned().collect();
    let max_name_len = app
        .entries
        .iter()
        .map(|item| item.name().len() + if item.is_dir() { 1 } else { 0 })
        .max()
        .unwrap_or(0);

    app.entries
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
            label
        })
        .collect()
}

fn build_help_lines() -> Vec<String> {
    vec![
        "Navigation".to_string(),
        "  ↑/↓          select".to_string(),
        "  PgUp/PgDn    page".to_string(),
        "  Enter        play/enter dir".to_string(),
        "  Backspace    parent dir".to_string(),
        "".to_string(),
        "Playback".to_string(),
        "  Space        pause/resume".to_string(),
        "  s            stop".to_string(),
        "  n            next".to_string(),
        "  ←/→          seek −5s / +5s".to_string(),
        "  Shift+←/→    seek −30s / +30s".to_string(),
        "".to_string(),
        "Queue".to_string(),
        "  k            queue selected track".to_string(),
        "  K            queue all tracks in dir".to_string(),
        "  c            clear queue".to_string(),
        "".to_string(),
        "Other".to_string(),
        "  p            jump to playing".to_string(),
        "  r            rescan".to_string(),
        "  o            outputs".to_string(),
        "  l            logs".to_string(),
        "  i            now playing".to_string(),
        "  h or ?       help".to_string(),
        "  q            quit".to_string(),
        "  Esc          close modal".to_string(),
    ]
}

fn build_logs_view(app: &App) -> (String, bool) {
    let title = "Logs (Esc to close, ↑/↓ scroll)".to_string();
    let total = app.logs.len();
    if total == 0 {
        return (title, true);
    }
    (title, false)
}

fn build_active_modal(
    app: &App,
    help_lines: &[String],
    outputs_title: &str,
    outputs_labels: &[String],
    outputs_empty: bool,
    outputs_error: Option<String>,
    logs_title: &str,
    logs_empty: bool,
) -> Option<UiModal> {
    if app.logs_open {
        return Some(UiModal::Logs {
            title: logs_title.to_string(),
            empty: logs_empty,
            layout: ModalLayout { width_pct: 90, height_pct: 80 },
        });
    }
    if app.help_open {
        return Some(UiModal::Help {
            title: "Help".to_string(),
            body: help_lines.join("\n"),
            layout: ModalLayout { width_pct: 70, height_pct: 70 },
        });
    }
    if app.outputs_open {
        return Some(UiModal::Outputs {
            title: outputs_title.to_string(),
            items: outputs_labels.to_vec(),
            empty: outputs_empty,
            error: outputs_error,
            layout: ModalLayout { width_pct: 60, height_pct: 60 },
        });
    }
    if app.confirm_clear_queue {
        return Some(UiModal::ClearQueue {
            title: "Clear Queue".to_string(),
            body: "Clear entire queue?\n\nPress y to confirm, n to cancel".to_string(),
            layout: ModalLayout { width_pct: 40, height_pct: 25 },
        });
    }
    None
}

fn build_now_playing_summary(app: &App) -> Vec<String> {
    let playing_name = app
        .now_playing_path
        .as_ref()
        .and_then(|p| p.file_name().and_then(|s| s.to_str()))
        .unwrap_or("-");
    let playing_path = app
        .now_playing_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "-".into());
    let playing_album = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.album.clone())
        .unwrap_or_else(|| "-".into());
    let playing_artist = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.artist.clone())
        .unwrap_or_else(|| "-".into());
    let playing_format = app
        .now_playing_meta
        .as_ref()
        .and_then(|m| m.format.clone())
        .unwrap_or_else(|| "-".into());
    let playing_channels = app
        .remote_channels
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".into());
    let sr_line = match (app.remote_output_sample_rate, app.remote_resampling) {
        (Some(sr), Some(resampling)) => {
            if resampling {
                format!("rate: {sr} Hz (resampled)")
            } else {
                format!("rate: {sr} Hz")
            }
        }
        (Some(sr), None) => format!("rate: {sr} Hz"),
        _ => "rate: -".to_string(),
    };

    vec![
        format!("track: {playing_name}"),
        format!("path: {playing_path}"),
        format!("album: {playing_album}"),
        format!("artist: {playing_artist}"),
        format!("format: {playing_format}"),
        format!("channels: {playing_channels}"),
        sr_line,
    ]
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

pub(crate) fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins}:{secs:02}")
}
