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

use std::{fs, thread};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() -> Result<()> {
    let args = cli::Args::parse();
    let host = cpal::default_host();

    if args.list_devices {
        device::list_devices(&host)?;
        return Ok(());
    }

    let device = device::pick_device(&host, args.device.as_deref())?;
    eprintln!("Output device: {}", device.description()?);

    let config = device.default_output_config()?;
    eprintln!("Device default config: {:?}", config);
    let stream_config: cpal::StreamConfig = config.clone().into();

    match &args.cmd {
        cli::Command::Play { path } => {
            play_one_local(&device, &config, &stream_config, &args, path)?;
        }
        cli::Command::Listen { bind } => {
            let listener = TcpListener::bind(*bind).with_context(|| format!("bind {bind}"))?;
            eprintln!("Listening on {bind} (one file per connection) ...");

            loop {
                let stream = match net::accept_one(&listener) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Accept error: {e:#}");
                        continue;
                    }
                };

                if let Err(e) = play_one_network(&device, &config, &stream_config, &args, stream) {
                    eprintln!("Playback error: {e:#}");
                }

                eprintln!("Ready for next connection...");
            }
        }
    }

    Ok(())
}

fn play_one_local(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    path: &std::path::PathBuf,
) -> Result<()> {
    let (src_spec, srcq) = decode::start_streaming_decode(path, args.buffer_seconds)?;
    eprintln!(
        "Source: {}ch @ {} Hz (local file)",
        src_spec.channels.count(),
        src_spec.rate
    );

    play_decoded_source(device, config, stream_config, args, src_spec, srcq)
}

fn play_one_network(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    stream: std::net::TcpStream,
) -> Result<()> {
    let (info, source, paused, cancel, mut peer_tx) = net::recv_one_framed_file_as_media_source(stream)?;
    eprintln!("Incoming stream spooling to {:?}", info.temp_path);

    let (src_spec, srcq) = decode::start_streaming_decode_from_media_source(
        source,
        info.hint,
        args.buffer_seconds,
    )?;

    let track_info_payload = audio_bridge_proto::encode_track_info(
        src_spec.rate,
        src_spec.channels.count() as u16,
        None,
    );
    let _ = audio_bridge_proto::write_frame(
        &mut peer_tx,
        audio_bridge_proto::FrameKind::TrackInfo,
        &track_info_payload,
    );

    let result = play_decoded_source_with_pause_and_progress(
        device,
        config,
        stream_config,
        args,
        src_spec,
        srcq,
        paused,
        cancel,
        peer_tx,
    );

    if let Err(e) = std::fs::remove_file(&info.temp_path) {
        eprintln!("Temp cleanup warning: {e}");
    }

    result
}

fn play_decoded_source_with_pause_and_progress(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    src_spec: symphonia::core::audio::SignalSpec,
    srcq: std::sync::Arc<queue::SharedAudio>,
    paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mut peer_tx: std::net::TcpStream,
) -> Result<()> {
    let srcq_for_cancel = srcq.clone();

    let dst_rate = stream_config.sample_rate;
    let dstq = resample::start_resampler(
        srcq,
        src_spec,
        dst_rate,
        resample::ResampleConfig {
            chunk_frames: args.chunk_frames,
            buffer_seconds: args.buffer_seconds,
        },
    )?;

    let played_frames = std::sync::Arc::new(AtomicU64::new(0));
    let played_frames_thread = played_frames.clone();
    let paused_thread = paused.clone();

    // Stop flag for reporter so we never block accept() waiting for it.
    let stop_reporter = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_reporter_thread = stop_reporter.clone();

    // Progress reporter thread (receiver -> sender).
    peer_tx.set_nodelay(true).ok();
    let reporter = std::thread::spawn(move || {
        loop {
            if stop_reporter_thread.load(Ordering::Relaxed) {
                break;
            }

            let frames = played_frames_thread.load(Ordering::Relaxed);
            let is_paused = paused_thread.load(Ordering::Relaxed);

            let payload = audio_bridge_proto::encode_playback_pos(frames, is_paused);
            if audio_bridge_proto::write_frame(
                &mut peer_tx,
                audio_bridge_proto::FrameKind::PlaybackPos,
                &payload,
            )
                .is_err()
            {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });

    let stream = playback::build_output_stream(
        device,
        stream_config,
        config.sample_format(),
        &dstq,
        playback::PlaybackConfig {
            refill_max_frames: args.refill_max_frames,
            paused: Some(paused.clone()),
            played_frames: Some(played_frames),
        },
    )?;
    stream.play()?;

    let finished_normally = queue::wait_until_done_and_empty_or_cancel(&dstq, &cancel);

    if !finished_normally {
        paused.store(true, Ordering::Relaxed);
        srcq_for_cancel.close();
        dstq.close();
    }

    // Stop reporter regardless of normal finish vs cancel, then join it.
    stop_reporter.store(true, Ordering::Relaxed);
    let _ = reporter.join();

    std::thread::sleep(std::time::Duration::from_millis(100));
    Ok(())
}

fn play_decoded_source(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &cli::Args,
    src_spec: symphonia::core::audio::SignalSpec,
    srcq: std::sync::Arc<queue::SharedAudio>,
) -> Result<()> {
    let dst_rate = stream_config.sample_rate;
    let dstq = resample::start_resampler(
        srcq,
        src_spec,
        dst_rate,
        resample::ResampleConfig {
            chunk_frames: args.chunk_frames,
            buffer_seconds: args.buffer_seconds,
        },
    )?;
    eprintln!("Resampling to {} Hz", dst_rate);

    let stream = playback::build_output_stream(
        device,
        stream_config,
        config.sample_format(),
        &dstq,
        playback::PlaybackConfig {
            refill_max_frames: args.refill_max_frames,
            paused: None,
            played_frames: None,
        },
    )?;
    stream.play()?;

    queue::wait_until_done_and_empty(&dstq);

    thread::sleep(Duration::from_millis(100));
    Ok(())
}