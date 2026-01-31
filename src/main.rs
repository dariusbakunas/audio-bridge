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
    let (info, source) = net::recv_one_file_as_media_source(stream)?;
    eprintln!("Incoming stream spooling to {:?}", info.temp_path);

    let (src_spec, srcq) = decode::start_streaming_decode_from_media_source(
        source,
        info.hint,
        args.buffer_seconds,
    )?;

    eprintln!(
        "Source: {}ch @ {} Hz (network stream)",
        src_spec.channels.count(),
        src_spec.rate
    );

    let result = play_decoded_source(device, config, stream_config, args, src_spec, srcq);

    // Best-effort cleanup of the temp file after playback finishes.
    if let Err(e) = fs::remove_file(&info.temp_path) {
        eprintln!("Temp cleanup warning: {e}");
    }

    result
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
        },
    )?;
    stream.play()?;

    queue::wait_until_done_and_empty(&dstq);

    thread::sleep(Duration::from_millis(100));
    Ok(())
}