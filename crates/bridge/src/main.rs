//! Audio Bridge — a small CLI utility that decodes an audio file, resamples it to the
//! output device sample rate, and plays it via CPAL.
//!
//! ## Pipeline
//! 1. **Decode**: a background thread uses Symphonia to decode the input into interleaved `f32`.
//! 2. **Resample**: a background thread uses Rubato to convert to the device sample rate.
//! 3. **Playback**: the CPAL callback pulls resampled audio without blocking and writes to the device.
//!
//! Stages communicate via bounded queues (`queue::SharedAudio`) sized by `--buffer-seconds` to
//! provide underrun resistance.
//!
//! ## Modes
//! - `play`: play a local file.
//! - `listen`: accept a TCP connection, receive one file, and play it; then go back to listening.

mod cli;
mod decode;
mod device;
mod playback;
mod queue;
mod resample;
mod net;
mod pipeline;
mod status;
mod http_api;

use std::net::TcpListener;
use std::sync::atomic::AtomicU64;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait};
use pipeline::{PlaybackSessionOptions, play_decoded_source};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let args = cli::Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,bridge=info")
        }))
        .init();
    let host = cpal::default_host();
    let device_selected = std::sync::Arc::new(std::sync::Mutex::new(args.device.clone()));
    let status = status::BridgeStatus::shared();

    if args.list_devices {
        device::list_devices(&host)?;
        return Ok(());
    }

    let temp_dir = args.temp_dir.clone().unwrap_or_else(std::env::temp_dir);

    match net::cleanup_temp_files(&temp_dir) {
        Ok(0) => {}
        Ok(n) => tracing::info!(count = n, "cleaned up stale temp files"),
        Err(e) => tracing::warn!("temp cleanup warning: {e}"),
    }

    let temp_dir_for_signal = temp_dir.clone();
    let _ = ctrlc::set_handler(move || {
        let _ = net::cleanup_temp_files(&temp_dir_for_signal);
        std::process::exit(130);
    });

    match &args.cmd {
        cli::Command::Play { path } => {
            let selected = device_selected.lock().unwrap().clone();
            let device = device::pick_device(&host, selected.as_deref())?;
            tracing::info!(device = %device.description()?, "output device");
            play_one_local(&device, &args, path)?;
        }
        cli::Command::Listen { bind } => {
            let listener = TcpListener::bind(*bind).with_context(|| format!("bind {bind}"))?;
            tracing::info!(bind = %bind, "listening (one client; many tracks per connection)");
            let _http = http_api::spawn_http_server(
                args.http_bind,
                status.clone(),
                device_selected.clone(),
            );

            let current_conn: std::sync::Arc<std::sync::Mutex<Option<std::net::TcpStream>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            loop {
                let stream = match net::accept_one(&listener) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("accept error: {e:#}");
                        continue;
                    }
                };
                if let Ok(mut guard) = current_conn.lock() {
                    if let Some(prev) = guard.take() {
                        let _ = prev.shutdown(std::net::Shutdown::Both);
                        tracing::info!("previous client evicted");
                    }
                    if let Ok(clone) = stream.try_clone() {
                        *guard = Some(clone);
                    }
                }

                let device_ctl = net::DeviceControl {
                    selected: device_selected.clone(),
                };
                if let Err(e) = serve_one_client(&host, &device_ctl, &args, stream, &temp_dir, status.clone()) {
                    tracing::warn!("client session error: {e:#}");
                }

                tracing::info!("client disconnected; ready for next connection");
            }
        }
    }

    Ok(())
}

fn serve_one_client(
    host: &cpal::Host,
    device_ctl: &net::DeviceControl,
    args: &cli::Args,
    stream: std::net::TcpStream,
    temp_dir: &std::path::Path,
    status: std::sync::Arc<std::sync::Mutex<status::BridgeStatus>>,
) -> Result<()> {
    let session_rx = net::run_one_client(stream, temp_dir.to_path_buf())?;

    while let Ok(sess) = session_rx.recv() {
        if let Err(e) = play_one_network_session(host, device_ctl, args, sess, status.clone()) {
            tracing::warn!("session playback error: {e:#}");
        }
    }

    Ok(())
}

fn play_one_network_session(
    host: &cpal::Host,
    device_ctl: &net::DeviceControl,
    args: &cli::Args,
    sess: net::NetSession,
    status: std::sync::Arc<std::sync::Mutex<status::BridgeStatus>>,
) -> Result<()> {
    tracing::info!(path = ?sess.temp_path, "incoming stream spooling");

    let file_for_read = std::fs::OpenOptions::new()
        .read(true)
        .open(&sess.temp_path)
        .with_context(|| format!("open temp file for read {:?}", sess.temp_path))?;

    let source: Box<dyn symphonia::core::io::MediaSource> =
        Box::new(net::BlockingFileSource::new(file_for_read, sess.control.progress.clone()));

    let (src_spec, srcq, duration_ms) = decode::start_streaming_decode_from_media_source(
        source,
        sess.hint.clone(),
        args.buffer_seconds,
    )?;

    let selected = device_ctl.selected.lock().unwrap().clone();
    let device = device::pick_device(host, selected.as_deref())?;
    let config = device::pick_output_config(&device, Some(src_spec.rate))?;
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    if let Some(buf) = device::pick_buffer_size(&config) {
        stream_config.buffer_size = buf;
    }
    tracing::info!(device = %device.description()?, "output device");
    tracing::info!(
        source_rate_hz = src_spec.rate,
        output_rate_hz = stream_config.sample_rate,
        buffer_size = ?stream_config.buffer_size,
        "device output config"
    );

    let played_frames = std::sync::Arc::new(AtomicU64::new(0));
    let underrun_frames = std::sync::Arc::new(AtomicU64::new(0));
    let underrun_events = std::sync::Arc::new(AtomicU64::new(0));
    {
        if let Ok(mut s) = status.lock() {
            s.now_playing = Some("stream".to_string());
            s.device = device.description().ok().map(|d| d.to_string());
            s.sample_rate = Some(stream_config.sample_rate);
            s.channels = Some(src_spec.channels.count() as u16);
            s.duration_ms = duration_ms;
            s.played_frames = Some(played_frames.clone());
            s.paused_flag = Some(sess.control.paused.clone());
            s.underrun_frames = Some(underrun_frames.clone());
            s.underrun_events = Some(underrun_events.clone());
            s.buffer_size_frames = match stream_config.buffer_size {
                cpal::BufferSize::Fixed(frames) => Some(frames),
                cpal::BufferSize::Default => None,
            };
        }
    }

    let mut peer_tx = sess.peer_tx;

    let track_info_payload = audio_bridge_proto::encode_track_info(
        stream_config.sample_rate,
        src_spec.channels.count() as u16,
        duration_ms,
    );
    let _ = audio_bridge_proto::write_frame(
        &mut peer_tx,
        audio_bridge_proto::FrameKind::TrackInfo,
        &track_info_payload,
    );

    let result = play_decoded_source(
        &device,
        &config,
        &stream_config,
        args,
        src_spec,
        srcq,
        PlaybackSessionOptions {
            paused: Some(sess.control.paused),
            cancel: Some(sess.control.cancel),
            peer_tx: Some(peer_tx),
            played_frames: Some(played_frames),
            underrun_frames: Some(underrun_frames),
            underrun_events: Some(underrun_events),
        },
    );

    if let Err(e) = std::fs::remove_file(&sess.temp_path) {
        tracing::warn!("temp cleanup warning: {e}");
    }

    if let Ok(mut s) = status.lock() {
        s.clear_playback();
    }

    result
}

fn play_one_local(
    device: &cpal::Device,
    args: &cli::Args,
    path: &std::path::PathBuf,
) -> Result<()> {
    let (src_spec, srcq, _duration_ms) = decode::start_streaming_decode(path, args.buffer_seconds)?;
    let config = device::pick_output_config(device, Some(src_spec.rate))?;
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    if let Some(buf) = device::pick_buffer_size(&config) {
        stream_config.buffer_size = buf;
    }
    tracing::info!(
        channels = src_spec.channels.count(),
        rate_hz = src_spec.rate,
        "source (local file)"
    );

    play_decoded_source(
        device,
        &config,
        &stream_config,
        args,
        src_spec,
        srcq,
        PlaybackSessionOptions {
            paused: None,
            cancel: None,
            peer_tx: None,
            played_frames: None,
            underrun_frames: None,
            underrun_events: None,
        },
    )
}
