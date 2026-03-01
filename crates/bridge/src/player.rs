//! Bridge playback worker.
//!
//! Receives HTTP playback commands and streams audio via the audio-player pipeline.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::DeviceTrait;
use crossbeam_channel::{Receiver, Sender};
use symphonia::core::probe::Hint;

use crate::http_stream::{HttpRangeConfig, HttpRangeSource};
use crate::status::BridgeStatusState;
use audio_bridge_types::PlaybackEndReason;
use audio_player::config::PlaybackConfig;
use audio_player::decode;
use audio_player::device;
use audio_player::pipeline;

/// Commands accepted by the playback worker thread.
#[derive(Debug, Clone)]
pub(crate) enum PlayerCommand {
    Play {
        url: String,
        ext_hint: Option<String>,
        title: Option<String>,
        seek_ms: Option<u64>,
    },
    PauseToggle,
    Resume,
    Stop,
    Seek {
        ms: u64,
    },
    SetVolume {
        value: u8,
    },
    SetMute {
        muted: bool,
    },
}

/// Handle for sending commands to the playback worker.
#[derive(Clone)]
pub(crate) struct PlayerHandle {
    pub(crate) cmd_tx: Sender<PlayerCommand>,
}

/// Shared bridge volume state (user-facing percent + mute).
#[derive(Debug)]
pub(crate) struct BridgeVolumeState {
    value: Arc<AtomicU8>,
    muted: Arc<AtomicBool>,
}

impl BridgeVolumeState {
    pub(crate) fn new(value: u8, muted: bool) -> Self {
        Self {
            value: Arc::new(AtomicU8::new(value.min(100))),
            muted: Arc::new(AtomicBool::new(muted)),
        }
    }

    pub(crate) fn snapshot(&self) -> (u8, bool) {
        (
            self.value.load(Ordering::Relaxed),
            self.muted.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn set_value(&self, value: u8) {
        self.value.store(value.min(100), Ordering::Relaxed);
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub(crate) fn volume_percent_handle(&self) -> Arc<AtomicU8> {
        self.value.clone()
    }

    pub(crate) fn muted_handle(&self) -> Arc<AtomicBool> {
        self.muted.clone()
    }
}

struct CurrentTrack {
    url: String,
    ext_hint: Option<String>,
    title: Option<String>,
}

struct SessionHandle {
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

/// Spawn the playback worker thread.
pub(crate) fn spawn_player(
    device_selected: Arc<Mutex<Option<String>>>,
    exclusive_selected: Arc<Mutex<bool>>,
    status: Arc<Mutex<BridgeStatusState>>,
    volume: Arc<BridgeVolumeState>,
    playback: PlaybackConfig,
    tls_insecure: bool,
) -> PlayerHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        player_thread_main(
            device_selected,
            exclusive_selected,
            status,
            volume,
            playback,
            tls_insecure,
            cmd_rx,
        )
    });
    PlayerHandle { cmd_tx }
}

/// Main loop for the playback worker.
fn player_thread_main(
    device_selected: Arc<Mutex<Option<String>>>,
    exclusive_selected: Arc<Mutex<bool>>,
    status: Arc<Mutex<BridgeStatusState>>,
    volume: Arc<BridgeVolumeState>,
    playback: PlaybackConfig,
    tls_insecure: bool,
    cmd_rx: Receiver<PlayerCommand>,
) {
    let session_id = Arc::new(AtomicU64::new(0));
    let mut current: Option<CurrentTrack> = None;
    let mut session: Option<SessionHandle> = None;
    let mut paused = false;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            PlayerCommand::Stop => {
                cancel_session(&mut session);
                current = None;
                paused = false;
                if let Ok(mut s) = status.lock() {
                    s.end_reason = Some(PlaybackEndReason::Stopped);
                    s.clear_playback();
                }
            }
            PlayerCommand::PauseToggle => {
                paused = !paused;
                tracing::info!(paused, "bridge pause toggled");
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(paused, Ordering::Relaxed);
                }
            }
            PlayerCommand::Resume => {
                paused = false;
                tracing::info!(paused, "bridge resume set");
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(false, Ordering::Relaxed);
                }
            }
            PlayerCommand::Seek { ms } => {
                let Some(track) = current.as_ref() else {
                    continue;
                };
                let url = track.url.clone();
                let ext_hint = track.ext_hint.clone();
                let title = track.title.clone();
                start_new_session(
                    &device_selected,
                    &exclusive_selected,
                    &status,
                    &volume,
                    &playback,
                    tls_insecure,
                    &session_id,
                    &mut session,
                    url,
                    ext_hint,
                    title,
                    Some(ms),
                    paused,
                    false,
                );
            }
            PlayerCommand::Play {
                url,
                ext_hint,
                title,
                seek_ms,
            } => {
                tracing::info!(
                    url = %url,
                    title = title.as_deref().unwrap_or(""),
                    seek_ms = ?seek_ms,
                    "bridge play received"
                );
                preupdate_status_on_play(&status, title.as_ref().unwrap_or(&url));
                current = Some(CurrentTrack {
                    url: url.clone(),
                    ext_hint: ext_hint.clone(),
                    title: title.clone(),
                });
                paused = false;
                start_new_session(
                    &device_selected,
                    &exclusive_selected,
                    &status,
                    &volume,
                    &playback,
                    tls_insecure,
                    &session_id,
                    &mut session,
                    url,
                    ext_hint,
                    title,
                    seek_ms,
                    paused,
                    true,
                );
            }
            PlayerCommand::SetVolume { value } => {
                volume.set_value(value);
            }
            PlayerCommand::SetMute { muted } => {
                volume.set_muted(muted);
            }
        }
    }
}

fn preupdate_status_on_play(status: &Arc<Mutex<BridgeStatusState>>, now_playing: &str) {
    if let Ok(mut s) = status.lock() {
        s.clear_playback();
        s.end_reason = None;
        s.now_playing = Some(now_playing.to_string());
    }
}

/// Cancel the current playback session and join its thread.
fn cancel_session(session: &mut Option<SessionHandle>) {
    if let Some(sess) = session.take() {
        sess.cancel.store(true, Ordering::Relaxed);
        let _ = sess.join.join();
    }
}

/// Cancel the current playback session without blocking.
fn cancel_session_async(session: &mut Option<SessionHandle>) {
    if let Some(sess) = session.take() {
        sess.cancel.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let _ = sess.join.join();
        });
    }
}

#[allow(clippy::too_many_arguments)]
/// Start a new playback session for the current URL.
fn start_new_session(
    device_selected: &Arc<Mutex<Option<String>>>,
    exclusive_selected: &Arc<Mutex<bool>>,
    status: &Arc<Mutex<BridgeStatusState>>,
    volume: &Arc<BridgeVolumeState>,
    playback: &PlaybackConfig,
    tls_insecure: bool,
    session_id: &Arc<AtomicU64>,
    session: &mut Option<SessionHandle>,
    url: String,
    ext_hint: Option<String>,
    title: Option<String>,
    seek_ms: Option<u64>,
    paused: bool,
    wait_for_cancel: bool,
) {
    if wait_for_cancel {
        cancel_session(session);
    } else {
        cancel_session_async(session);
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let paused_flag = Arc::new(AtomicBool::new(paused));
    let my_id = session_id.fetch_add(1, Ordering::Relaxed).saturating_add(1);

    let device_selected = device_selected.clone();
    let exclusive_selected = exclusive_selected.clone();
    let status = status.clone();
    let volume = volume.clone();
    let playback = playback.clone();
    let session_id = session_id.clone();
    let cancel_for_thread = cancel.clone();
    let paused_for_thread = paused_flag.clone();

    let join = std::thread::spawn(move || {
        let host = cpal::default_host();
        if let Err(e) = play_one_http(
            &host,
            &device_selected,
            &exclusive_selected,
            &status,
            &volume,
            &playback,
            tls_insecure,
            url,
            ext_hint,
            title,
            seek_ms,
            cancel_for_thread,
            paused_for_thread,
            my_id,
            session_id,
        ) {
            tracing::warn!("http playback error: {e:#}");
        }
    });

    *session = Some(SessionHandle {
        cancel,
        paused: paused_flag,
        join,
    });
}

#[allow(clippy::too_many_arguments)]
/// Decode and play a remote HTTP source.
fn play_one_http(
    host: &cpal::Host,
    device_selected: &Arc<Mutex<Option<String>>>,
    exclusive_selected: &Arc<Mutex<bool>>,
    status: &Arc<Mutex<BridgeStatusState>>,
    volume: &Arc<BridgeVolumeState>,
    playback: &PlaybackConfig,
    tls_insecure: bool,
    url: String,
    ext_hint: Option<String>,
    title: Option<String>,
    seek_ms: Option<u64>,
    cancel: Arc<AtomicBool>,
    paused_flag: Arc<AtomicBool>,
    my_id: u64,
    session_id: Arc<AtomicU64>,
) -> Result<()> {
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint
        .as_ref()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
    {
        hint.with_extension(ext);
    } else if let Some(ext) = infer_ext_from_url(&url) {
        hint.with_extension(&ext);
    }

    let playback_eff = effective_playback_for_seek(playback, seek_ms);

    tracing::debug!(
        url = %url,
        tls_insecure,
        "bridge http stream start"
    );
    let stream_error = Arc::new(AtomicBool::new(false));
    let source = HttpRangeSource::new(
        url.clone(),
        HttpRangeConfig {
            tls_insecure,
            ..HttpRangeConfig::default()
        },
        Some(cancel.clone()),
        Some(stream_error.clone()),
    );
    let (src_spec, srcq, duration_ms, source_info) =
        decode::start_streaming_decode_from_media_source_at(
            Box::new(source),
            hint,
            playback_eff.buffer_seconds,
            seek_ms,
        )
        .context("decode from http")?;

    let selected = device_selected.lock().unwrap().clone();
    let device = device::pick_device(host, selected.as_deref())?;
    let exclusive_mode = exclusive_selected.lock().map(|g| *g).unwrap_or(false);
    let _exclusive = crate::exclusive::maybe_acquire(&device, src_spec.rate, exclusive_mode);
    let nominal_rate = crate::exclusive::current_nominal_rate(&device);
    let config = device::pick_output_config(&device, Some(src_spec.rate))?;
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    if let Some(buf) = device::pick_buffer_size(&config) {
        stream_config.buffer_size = buf;
    }

    let played_frames = Arc::new(AtomicU64::new(0));
    if let Some(ms) = seek_ms {
        if let Some(frames) = played_frames_from_seek(ms, duration_ms, stream_config.sample_rate) {
            played_frames.store(frames, Ordering::Relaxed);
        }
    }
    let underrun_frames = Arc::new(AtomicU64::new(0));
    let underrun_events = Arc::new(AtomicU64::new(0));
    let buffered_frames = Arc::new(AtomicU64::new(0));
    let buffer_capacity_frames = Arc::new(AtomicU64::new(0));
    let output_sample_format = Some(format!("{:?}", config.sample_format()));
    let container = ext_hint
        .clone()
        .or_else(|| infer_ext_from_url(&url))
        .map(|s| s.to_ascii_uppercase());
    let resampling = src_spec.rate != stream_config.sample_rate;
    {
        if let Ok(mut s) = status.lock() {
            s.end_reason = None;
            s.now_playing = Some(title.clone().unwrap_or_else(|| url.clone()));
            s.device = device.description().ok().map(|d| d.to_string());
            s.sample_rate = Some(status_sample_rate(stream_config.sample_rate, nominal_rate));
            s.channels = Some(src_spec.channels.count() as u16);
            s.duration_ms = duration_ms;
            s.source_codec = source_info.codec.clone();
            s.source_bit_depth = source_info.bit_depth;
            s.container = container.or_else(|| source_info.container.clone());
            s.output_sample_format = output_sample_format.clone();
            s.resampling = Some(resampling);
            s.resample_from_hz = Some(src_spec.rate);
            s.resample_to_hz = Some(stream_config.sample_rate);
            s.played_frames = Some(played_frames.clone());
            s.paused_flag = Some(paused_flag.clone());
            s.underrun_frames = Some(underrun_frames.clone());
            s.underrun_events = Some(underrun_events.clone());
            s.buffer_size_frames = match stream_config.buffer_size {
                cpal::BufferSize::Fixed(frames) => Some(frames),
                cpal::BufferSize::Default => None,
            };
            s.buffered_frames = Some(buffered_frames.clone());
            s.buffer_capacity_frames = Some(buffer_capacity_frames.clone());
        }
    }
    tracing::info!(
        url = %url,
        seek_ms = ?seek_ms,
        paused = paused_flag.load(Ordering::Relaxed),
        "bridge status updated from decoder"
    );

    let cancel_for_status = cancel.clone();
    let stream_error_for_status = stream_error.clone();
    let result = pipeline::play_decoded_source(
        &device,
        &config,
        &stream_config,
        &playback_eff,
        src_spec,
        srcq,
        pipeline::PlaybackSessionOptions {
            paused: Some(paused_flag),
            cancel: Some(cancel),
            played_frames: Some(played_frames),
            underrun_frames: Some(underrun_frames),
            underrun_events: Some(underrun_events),
            buffered_frames: Some(buffered_frames),
            buffer_capacity_frames: Some(buffer_capacity_frames),
            volume_percent: Some(volume.volume_percent_handle()),
            muted: Some(volume.muted_handle()),
        },
    );

    if session_id.load(Ordering::Relaxed) == my_id {
        if let Ok(mut s) = status.lock() {
            let should_set = s.end_reason.is_none();
            if should_set {
                let cancelled = cancel_for_status.load(Ordering::Relaxed);
                let had_error = stream_error_for_status.load(Ordering::Relaxed);
                s.end_reason = Some(if result.is_ok() && !cancelled && !had_error {
                    PlaybackEndReason::Eof
                } else {
                    PlaybackEndReason::Error
                });
            }
            s.clear_playback();
        }
    }

    result
}

fn effective_playback_for_seek(playback: &PlaybackConfig, seek_ms: Option<u64>) -> PlaybackConfig {
    let mut playback_eff = playback.clone();
    if seek_ms.is_some() {
        playback_eff.buffer_seconds = playback_eff.buffer_seconds.min(1.0);
        playback_eff.refill_max_frames = playback_eff.refill_max_frames.min(2048);
        playback_eff.chunk_frames = playback_eff.chunk_frames.min(1024);
    }
    playback_eff
}

fn played_frames_from_seek(
    seek_ms: u64,
    duration_ms: Option<u64>,
    sample_rate_hz: u32,
) -> Option<u64> {
    if sample_rate_hz == 0 {
        return None;
    }
    let target_ms = duration_ms.map_or(seek_ms, |total| seek_ms.min(total));
    Some(target_ms.saturating_mul(sample_rate_hz as u64) / 1000)
}

fn status_sample_rate(stream_sample_rate: u32, _nominal_rate: Option<u32>) -> u32 {
    // Elapsed time is derived from played_frames / sample_rate. We must use the
    // actual stream sample rate (not hardware nominal rate) to keep elapsed_ms
    // and seek restoration accurate.
    stream_sample_rate
}

/// Infer a file extension from the URL path if present.
fn infer_ext_from_url(url: &str) -> Option<String> {
    let tail = url.split('?').next().unwrap_or(url);
    let file = tail.rsplit('/').next().unwrap_or(tail);
    let mut parts = file.rsplit('.');
    let ext = parts.next()?;
    if parts.next().is_some() {
        Some(ext.to_ascii_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_playback_for_seek_caps_values() {
        let playback = PlaybackConfig {
            buffer_seconds: 3.0,
            refill_max_frames: 8192,
            chunk_frames: 4096,
        };
        let eff = effective_playback_for_seek(&playback, Some(1000));
        assert_eq!(eff.buffer_seconds, 1.0);
        assert_eq!(eff.refill_max_frames, 2048);
        assert_eq!(eff.chunk_frames, 1024);
    }

    #[test]
    fn effective_playback_for_seek_keeps_values_without_seek() {
        let playback = PlaybackConfig {
            buffer_seconds: 2.5,
            refill_max_frames: 4096,
            chunk_frames: 2048,
        };
        let eff = effective_playback_for_seek(&playback, None);
        assert_eq!(eff.buffer_seconds, 2.5);
        assert_eq!(eff.refill_max_frames, 4096);
        assert_eq!(eff.chunk_frames, 2048);
    }

    #[test]
    fn infer_ext_from_url_handles_query_and_missing_ext() {
        assert_eq!(
            infer_ext_from_url("http://example/a.flac?x=1"),
            Some("flac".to_string())
        );
        assert_eq!(infer_ext_from_url("http://example/a"), None);
    }

    #[test]
    fn infer_ext_from_url_handles_multiple_dots() {
        assert_eq!(
            infer_ext_from_url("http://example/archive.track.flac"),
            Some("flac".to_string())
        );
    }

    #[test]
    fn played_frames_from_seek_clamps_to_duration() {
        let frames = played_frames_from_seek(5_000, Some(2_000), 48_000).unwrap();
        assert_eq!(frames, 96_000);
    }

    #[test]
    fn played_frames_from_seek_uses_seek_when_duration_missing() {
        let frames = played_frames_from_seek(1_500, None, 44_100).unwrap();
        assert_eq!(frames, 66_150);
    }

    #[test]
    fn played_frames_from_seek_returns_none_for_zero_rate() {
        assert!(played_frames_from_seek(1_000, Some(2_000), 0).is_none());
    }

    #[test]
    fn status_sample_rate_prefers_stream_rate_over_nominal() {
        assert_eq!(status_sample_rate(44_100, Some(96_000)), 44_100);
        assert_eq!(status_sample_rate(48_000, None), 48_000);
    }
}
