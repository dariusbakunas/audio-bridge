use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use anyhow::{anyhow, Context, Result};
use symphonia::core::{
    audio::SignalSpec,
    codecs::DecoderOptions,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::core::audio::{SampleBuffer, Signal};

use crate::queue::SharedAudio;

pub fn start_streaming_decode(path: &PathBuf) -> Result<(SignalSpec, Arc<SharedAudio>)> {
    // Probe once to get spec.
    let file = File::open(path).with_context(|| format!("open {:?}", path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("No default audio track"))?;

    let channels = track
        .codec_params
        .channels
        .ok_or_else(|| anyhow!("Unknown channels"))?
        .count();

    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;

    let spec = SignalSpec::new(rate, track.codec_params.channels.unwrap());
    let max_buffered_samples = (rate as usize).saturating_mul(channels).saturating_mul(2);

    let shared = Arc::new(SharedAudio::new(channels, max_buffered_samples));

    let path_for_thread = path.clone();
    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        if let Err(e) = decode_thread_main(&path_for_thread, &shared_for_thread) {
            eprintln!("Decoder thread error: {e:#}");
        }
        shared_for_thread.close();
    });

    Ok((spec, shared))
}

fn decode_thread_main(path: &PathBuf, shared: &Arc<SharedAudio>) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open {:?}", path))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("No default audio track"))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break, // EOF
        };

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut sample_buf = SampleBuffer::<f32>::new(decoded.frames() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);

        shared.push_interleaved_blocking(sample_buf.samples());
    }

    Ok(())
}