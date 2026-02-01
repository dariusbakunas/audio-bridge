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

use std::net::TcpListener;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait};
use pipeline::{PlaybackSessionOptions, play_decoded_source};

fn main() -> Result<()> {
    let args = cli::Args::parse();
    let host = cpal::default_host();

    if args.list_devices {
        device::list_devices(&host)?;
        return Ok(());
    }

    let temp_dir = args.temp_dir.clone().unwrap_or_else(std::env::temp_dir);

    match net::cleanup_temp_files(&temp_dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("Cleaned up {n} stale temp file(s)."),
        Err(e) => eprintln!("Temp cleanup warning: {e}"),
    }

    let temp_dir_for_signal = temp_dir.clone();
    let _ = ctrlc::set_handler(move || {
        let _ = net::cleanup_temp_files(&temp_dir_for_signal);
        std::process::exit(130);
    });

    let (device, config) = device::select_output(&host, args.device.as_deref())?;
    eprintln!("Output device: {}", device.description()?);
    eprintln!("Device default config: {:?}", config);
    let stream_config: cpal::StreamConfig = config.clone().into();

    match &args.cmd {
        cli::Command::Play { path } => {
            play_one_local(&device, &config, &stream_config, &args, path)?;
        }
        cli::Command::Listen { bind } => {
            let listener = TcpListener::bind(*bind).with_context(|| format!("bind {bind}"))?;
            eprintln!("Listening on {bind} (one client; many tracks per connection) ...");

            loop {
                let stream = match net::accept_one(&listener) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Accept error: {e:#}");
                        continue;
                    }
                };

                if let Err(e) = serve_one_client(&device, &config, &stream_config, &args, stream, &temp_dir) {
                    eprintln!("Client session error: {e:#}");
                }

                eprintln!("Client disconnected; ready for next connection...");
            }
        }
    }

    Ok(())
}

fn serve_one_client(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    stream: std::net::TcpStream,
    temp_dir: &std::path::Path,
) -> Result<()> {
    let session_rx = net::run_one_client(stream, temp_dir.to_path_buf())?;

    while let Ok(sess) = session_rx.recv() {
        if let Err(e) = play_one_network_session(device, config, stream_config, args, sess) {
            eprintln!("Session playback error: {e:#}");
        }
    }

    Ok(())
}

fn play_one_network_session(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    sess: net::NetSession,
) -> Result<()> {
    eprintln!("Incoming stream spooling to {:?}", sess.temp_path);

    let file_for_read = std::fs::OpenOptions::new()
        .read(true)
        .open(&sess.temp_path)
        .with_context(|| format!("open temp file for read {:?}", sess.temp_path))?;

    // Reuse your existing blocking MediaSource idea.
    let source: Box<dyn symphonia::core::io::MediaSource> =
        Box::new(net::BlockingFileSource::new(file_for_read, sess.control.progress.clone()));

    let (src_spec, srcq, duration_ms) = decode::start_streaming_decode_from_media_source(
        source,
        sess.hint.clone(),
        args.buffer_seconds,
    )?;

    let track_info_payload = audio_bridge_proto::encode_track_info(
        stream_config.sample_rate,
        src_spec.channels.count() as u16,
        duration_ms,
    );
    let mut peer_tx = sess.peer_tx;
    let _ = audio_bridge_proto::write_frame(
        &mut peer_tx,
        audio_bridge_proto::FrameKind::TrackInfo,
        &track_info_payload,
    );

    let result = play_decoded_source(
        device,
        config,
        stream_config,
        args,
        src_spec,
        srcq,
        PlaybackSessionOptions {
            paused: Some(sess.control.paused),
            cancel: Some(sess.control.cancel),
            peer_tx: Some(peer_tx),
        },
    );

    if let Err(e) = std::fs::remove_file(&sess.temp_path) {
        eprintln!("Temp cleanup warning: {e}");
    }

    result
}

fn play_one_local(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    path: &std::path::PathBuf,
) -> Result<()> {
    let (src_spec, srcq, _duration_ms) = decode::start_streaming_decode(path, args.buffer_seconds)?;
    eprintln!(
        "Source: {}ch @ {} Hz (local file)",
        src_spec.channels.count(),
        src_spec.rate
    );

    play_decoded_source(
        device,
        config,
        stream_config,
        args,
        src_spec,
        srcq,
        PlaybackSessionOptions {
            paused: None,
            cancel: None,
            peer_tx: None,
        },
    )
}
