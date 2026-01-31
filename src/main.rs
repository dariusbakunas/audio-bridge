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
//! ## Tuning
//! - `--buffer-seconds`: larger values increase stability but add latency.
//! - `--chunk-frames`: resampler processing granularity.
//! - `--refill-max-frames`: how much the audio callback tries to pull per refill.
//!
//! ## Notes
//! This tool aims for stable playback on typical USB DAC setups. Underruns are rendered as silence.

mod cli;
mod decode;
mod device;
mod playback;
mod queue;
mod resample;

use std::thread;
use std::time::Duration;

use anyhow::Result;
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

    let (src_spec, srcq) = decode::start_streaming_decode(&args.path, args.buffer_seconds)?;
    eprintln!(
        "Source: {}ch @ {} Hz (streaming decode)",
        src_spec.channels.count(),
        src_spec.rate
    );

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
        &device,
        &stream_config,
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