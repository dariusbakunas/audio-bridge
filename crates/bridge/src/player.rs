//! HTTP-controlled playback manager.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use cpal::traits::DeviceTrait;
use symphonia::core::probe::Hint;

use crate::config::PlaybackConfig;
use crate::decode;
use crate::device;
use crate::http_stream::{HttpRangeConfig, HttpRangeSource};
use crate::pipeline;
use crate::status::BridgeStatus;

#[derive(Debug, Clone)]
pub(crate) enum PlayerCommand {
    Play {
        url: String,
        ext_hint: Option<String>,
        title: Option<String>,
        seek_ms: Option<u64>,
    },
    PauseToggle,
    Pause,
    Resume,
    Stop,
    Seek { ms: u64 },
    Quit,
}

#[derive(Clone)]
pub(crate) struct PlayerHandle {
    pub(crate) cmd_tx: Sender<PlayerCommand>,
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

pub(crate) fn spawn_player(
    device_selected: Arc<Mutex<Option<String>>>,
    status: Arc<Mutex<BridgeStatus>>,
    playback: PlaybackConfig,
) -> PlayerHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || player_thread_main(device_selected, status, playback, cmd_rx));
    PlayerHandle { cmd_tx }
}

fn player_thread_main(
    device_selected: Arc<Mutex<Option<String>>>,
    status: Arc<Mutex<BridgeStatus>>,
    playback: PlaybackConfig,
    cmd_rx: Receiver<PlayerCommand>,
) {
    let session_id = Arc::new(AtomicU64::new(0));
    let mut current: Option<CurrentTrack> = None;
    let mut session: Option<SessionHandle> = None;
    let mut paused = false;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            PlayerCommand::Quit => {
                cancel_session(&mut session);
                break;
            }
            PlayerCommand::Stop => {
                cancel_session(&mut session);
                current = None;
                paused = false;
                if let Ok(mut s) = status.lock() {
                    s.clear_playback();
                }
            }
            PlayerCommand::PauseToggle => {
                paused = !paused;
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(paused, Ordering::Relaxed);
                }
            }
            PlayerCommand::Pause => {
                paused = true;
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(true, Ordering::Relaxed);
                }
            }
            PlayerCommand::Resume => {
                paused = false;
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(false, Ordering::Relaxed);
                }
            }
            PlayerCommand::Seek { ms } => {
                let Some(track) = current.as_ref() else { continue };
                let url = track.url.clone();
                let ext_hint = track.ext_hint.clone();
                let title = track.title.clone();
                start_new_session(
                    &device_selected,
                    &status,
                    &playback,
                    &session_id,
                    &mut session,
                    url,
                    ext_hint,
                    title,
                    Some(ms),
                    paused,
                );
            }
            PlayerCommand::Play {
                url,
                ext_hint,
                title,
                seek_ms,
            } => {
                current = Some(CurrentTrack {
                    url: url.clone(),
                    ext_hint: ext_hint.clone(),
                    title: title.clone(),
                });
                paused = false;
                start_new_session(
                    &device_selected,
                    &status,
                    &playback,
                    &session_id,
                    &mut session,
                    url,
                    ext_hint,
                    title,
                    seek_ms,
                    paused,
                );
            }
        }
    }
}

fn cancel_session(session: &mut Option<SessionHandle>) {
    if let Some(sess) = session.take() {
        sess.cancel.store(true, Ordering::Relaxed);
        let _ = sess.join.join();
    }
}

#[allow(clippy::too_many_arguments)]
fn start_new_session(
    device_selected: &Arc<Mutex<Option<String>>>,
    status: &Arc<Mutex<BridgeStatus>>,
    playback: &PlaybackConfig,
    session_id: &Arc<AtomicU64>,
    session: &mut Option<SessionHandle>,
    url: String,
    ext_hint: Option<String>,
    title: Option<String>,
    seek_ms: Option<u64>,
    paused: bool,
) {
    cancel_session(session);

    let cancel = Arc::new(AtomicBool::new(false));
    let paused_flag = Arc::new(AtomicBool::new(paused));
    let my_id = session_id.fetch_add(1, Ordering::Relaxed).saturating_add(1);

    let device_selected = device_selected.clone();
    let status = status.clone();
    let playback = playback.clone();
    let session_id = session_id.clone();
    let cancel_for_thread = cancel.clone();
    let paused_for_thread = paused_flag.clone();

    let join = std::thread::spawn(move || {
        let host = cpal::default_host();
        if let Err(e) = play_one_http(
            &host,
            &device_selected,
            &status,
            &playback,
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
fn play_one_http(
    host: &cpal::Host,
    device_selected: &Arc<Mutex<Option<String>>>,
    status: &Arc<Mutex<BridgeStatus>>,
    playback: &PlaybackConfig,
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
    if let Some(ext) = ext_hint.as_ref().and_then(|s| if s.is_empty() { None } else { Some(s) }) {
        hint.with_extension(ext);
    } else if let Some(ext) = infer_ext_from_url(&url) {
        hint.with_extension(&ext);
    }

    let mut playback_eff = playback.clone();
    if seek_ms.is_some() {
        playback_eff.buffer_seconds = playback_eff.buffer_seconds.min(1.0);
        playback_eff.refill_max_frames = playback_eff.refill_max_frames.min(2048);
        playback_eff.chunk_frames = playback_eff.chunk_frames.min(1024);
    }

    let source = HttpRangeSource::new(url.clone(), HttpRangeConfig::default(), Some(cancel.clone()));
    let (src_spec, srcq, duration_ms) =
        decode::start_streaming_decode_from_media_source_at(
            Box::new(source),
            hint,
            playback_eff.buffer_seconds,
            seek_ms,
        )
        .context("decode from http")?;

    let selected = device_selected.lock().unwrap().clone();
    let device = device::pick_device(host, selected.as_deref())?;
    let config = device::pick_output_config(&device, Some(src_spec.rate))?;
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    if let Some(buf) = device::pick_buffer_size(&config) {
        stream_config.buffer_size = buf;
    }

    let played_frames = Arc::new(AtomicU64::new(0));
    if let Some(ms) = seek_ms {
        let mut target_ms = ms;
        if let Some(total) = duration_ms {
            if target_ms > total {
                target_ms = total;
            }
        }
        if stream_config.sample_rate > 0 {
            let frames = target_ms.saturating_mul(stream_config.sample_rate as u64) / 1000;
            played_frames.store(frames, Ordering::Relaxed);
        }
    }
    let underrun_frames = Arc::new(AtomicU64::new(0));
    let underrun_events = Arc::new(AtomicU64::new(0));
    {
        if let Ok(mut s) = status.lock() {
            s.now_playing = Some(title.clone().unwrap_or_else(|| url.clone()));
            s.device = device.description().ok().map(|d| d.to_string());
            s.sample_rate = Some(stream_config.sample_rate);
            s.channels = Some(src_spec.channels.count() as u16);
            s.duration_ms = duration_ms;
            s.played_frames = Some(played_frames.clone());
            s.paused_flag = Some(paused_flag.clone());
            s.underrun_frames = Some(underrun_frames.clone());
            s.underrun_events = Some(underrun_events.clone());
            s.buffer_size_frames = match stream_config.buffer_size {
                cpal::BufferSize::Fixed(frames) => Some(frames),
                cpal::BufferSize::Default => None,
            };
        }
    }

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
        },
    );

    if session_id.load(Ordering::Relaxed) == my_id {
        if let Ok(mut s) = status.lock() {
            s.clear_playback();
        }
    }

    result
}

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
