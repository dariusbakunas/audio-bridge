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

    let (src_spec, srcq) = decode::start_streaming_decode(&args.path)?;
    eprintln!(
        "Source: {}ch @ {} Hz (streaming decode)",
        src_spec.channels.count(),
        src_spec.rate
    );

    let dst_rate = stream_config.sample_rate;
    let dstq = resample::start_resampler(srcq, src_spec, dst_rate)?;
    eprintln!("Resampling to {} Hz", dst_rate);

    let stream = playback::build_output_stream(
        &device,
        &stream_config,
        config.sample_format(),
        dstq.clone(),
    )?;
    stream.play()?;

    queue::wait_until_done_and_empty(&dstq);

    thread::sleep(Duration::from_millis(100));
    Ok(())
}