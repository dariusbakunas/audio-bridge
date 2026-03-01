//! Local playback worker.
//!
//! Uses `audio-player` to decode and play files on the host machine.

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::DeviceTrait;
use crossbeam_channel::{Receiver, Sender};
use symphonia::core::probe::Hint;

use audio_player::config::PlaybackConfig;
use audio_player::{decode, device, pipeline};

use crate::bridge::BridgeCommand;
use crate::status_store::StatusStore;

/// Handle for sending playback commands to the local player thread.
#[derive(Clone)]
pub(crate) struct LocalPlayerHandle {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

struct CurrentTrack {
    path: PathBuf,
}

struct SessionHandle {
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

/// Spawn the local playback worker thread.
pub(crate) fn spawn_local_player(
    device_selected: Arc<Mutex<Option<String>>>,
    status: StatusStore,
    playback: PlaybackConfig,
) -> LocalPlayerHandle {
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || player_thread_main(device_selected, status, playback, cmd_rx));
    LocalPlayerHandle { cmd_tx }
}

/// Main command loop for local playback worker.
fn player_thread_main(
    device_selected: Arc<Mutex<Option<String>>>,
    status: StatusStore,
    playback: PlaybackConfig,
    cmd_rx: Receiver<BridgeCommand>,
) {
    let session_id = Arc::new(AtomicU64::new(0));
    let mut current: Option<CurrentTrack> = None;
    let mut session: Option<SessionHandle> = None;
    let mut paused = false;

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            BridgeCommand::Quit => {
                cancel_session(&mut session);
                break;
            }
            BridgeCommand::Stop => {
                cancel_session(&mut session);
                current = None;
                paused = false;
                status.on_stop();
            }
            BridgeCommand::StopSilent => {
                cancel_session(&mut session);
                current = None;
                paused = false;
            }
            BridgeCommand::PauseToggle => {
                paused = !paused;
                if let Some(sess) = session.as_ref() {
                    sess.paused.store(paused, Ordering::Relaxed);
                }
                status.on_pause_toggle();
            }
            BridgeCommand::Seek { ms } => {
                let Some(track) = current.as_ref() else {
                    continue;
                };
                status.mark_seek_in_flight();
                start_new_session(
                    &device_selected,
                    &status,
                    &playback,
                    &session_id,
                    &mut session,
                    track.path.clone(),
                    Some(ms),
                    paused,
                );
            }
            BridgeCommand::Play {
                path,
                seek_ms,
                start_paused,
                ..
            } => {
                current = Some(CurrentTrack { path: path.clone() });
                paused = start_paused;
                start_new_session(
                    &device_selected,
                    &status,
                    &playback,
                    &session_id,
                    &mut session,
                    path,
                    seek_ms,
                    paused,
                );
            }
        }
    }
}

/// Cancel currently running local playback session and join its thread.
fn cancel_session(session: &mut Option<SessionHandle>) {
    if let Some(sess) = session.take() {
        sess.cancel.store(true, Ordering::Relaxed);
        let _ = sess.join.join();
    }
}

#[allow(clippy::too_many_arguments)]
fn start_new_session(
    device_selected: &Arc<Mutex<Option<String>>>,
    status: &StatusStore,
    playback: &PlaybackConfig,
    session_id: &Arc<AtomicU64>,
    session: &mut Option<SessionHandle>,
    path: PathBuf,
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
        if let Err(e) = play_one_file(
            &host,
            &device_selected,
            &status,
            &playback,
            path,
            seek_ms,
            cancel_for_thread,
            paused_for_thread,
            my_id,
            session_id,
        ) {
            tracing::warn!("local playback error: {e:#}");
        }
    });

    *session = Some(SessionHandle {
        cancel,
        paused: paused_flag,
        join,
    });
}

/// Decode and play one local file on selected output device.
fn play_one_file(
    host: &cpal::Host,
    device_selected: &Arc<Mutex<Option<String>>>,
    status: &StatusStore,
    playback: &PlaybackConfig,
    path: PathBuf,
    seek_ms: Option<u64>,
    cancel: Arc<AtomicBool>,
    paused_flag: Arc<AtomicBool>,
    my_id: u64,
    session_id: Arc<AtomicU64>,
) -> Result<()> {
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let mut playback_eff = playback.clone();
    if seek_ms.is_some() {
        playback_eff.buffer_seconds = playback_eff.buffer_seconds.min(1.0);
        playback_eff.refill_max_frames = playback_eff.refill_max_frames.min(2048);
        playback_eff.chunk_frames = playback_eff.chunk_frames.min(1024);
    }

    let file = File::open(&path).with_context(|| format!("open {:?}", path))?;
    let (src_spec, srcq, duration_ms, source_info) =
        decode::start_streaming_decode_from_media_source_at(
            Box::new(file),
            hint,
            playback_eff.buffer_seconds,
            seek_ms,
        )
        .context("decode local file")?;

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
    let output_sample_format = Some(format!("{:?}", config.sample_format()));
    let container = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_uppercase());
    let resampling = src_spec.rate != stream_config.sample_rate;
    status.on_local_playback_start(
        path.clone(),
        device.description().ok().map(|d| d.to_string()),
        stream_config.sample_rate,
        src_spec.channels.count() as u16,
        duration_ms,
        source_info.codec.clone(),
        source_info.bit_depth,
        container.or_else(|| source_info.container.clone()),
        output_sample_format.clone(),
        resampling,
        src_spec.rate,
        stream_config.sample_rate,
        seek_ms,
        paused_flag.load(Ordering::Relaxed),
    );

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
            buffered_frames: None,
            buffer_capacity_frames: None,
            volume_percent: None,
            muted: None,
        },
    );

    if session_id.load(Ordering::Relaxed) == my_id {
        status.on_local_playback_end();
    }

    result
}
