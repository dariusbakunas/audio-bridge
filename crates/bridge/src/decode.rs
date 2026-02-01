//! Streaming audio decode stage.
//!
//! Uses Symphonia to:
//! - probe the input container/codec
//! - decode packets into interleaved `f32` samples
//! - push samples into a bounded [`SharedAudio`] queue from a background thread


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
use symphonia::core::audio::{SampleBuffer};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::io::MediaSource;
use crate::queue::{calc_max_buffered_samples, SharedAudio};

/// Start decoding from an arbitrary Symphonia [`MediaSource`] (seekable or not).
///
/// This is the shared entry point used by both:
/// - local file playback
/// - network-spooled playback
///
/// The queue is closed on EOF or error.
pub(crate) fn start_streaming_decode_from_media_source(
    source: Box<dyn MediaSource>,
    hint: Hint,
    buffer_seconds: f32,
) -> Result<(SignalSpec, Arc<SharedAudio>, Option<u64>)> {
    // Probe once to get spec.
    let mss = MediaSourceStream::new(source, Default::default());

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let format = probed.format;
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

    let codec_params: CodecParameters = track.codec_params.clone();
    let duration_ms = duration_ms_from_codec_params(&codec_params);

    let max_buffered_samples = calc_max_buffered_samples(rate, channels, buffer_seconds);
    let shared = Arc::new(SharedAudio::new(channels, max_buffered_samples));

    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        if let Err(e) = decode_format_loop(format, codec_params, &shared_for_thread) {
            tracing::error!("decoder thread error: {e:#}");
        }
        shared_for_thread.close();
    });

    Ok((spec, shared, duration_ms))
}

/// Start a background decoder thread that streams interleaved `f32` samples from `path`.
pub(crate) fn start_streaming_decode(
    path: &PathBuf,
    buffer_seconds: f32,
) -> Result<(SignalSpec, Arc<SharedAudio>, Option<u64>)> {
    let file = File::open(path).with_context(|| format!("open {:?}", path))?;

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    start_streaming_decode_from_media_source(Box::new(file), hint, buffer_seconds)
}

/// Decode packets from a probed `FormatReader` and push interleaved `f32` into `shared`.
///
/// This runs in the background thread spawned by `start_streaming_decode_from_media_source`.
fn decode_format_loop(
    mut format: Box<dyn symphonia::core::formats::FormatReader>,
    codec_params: CodecParameters,
    shared: &Arc<SharedAudio>,
) -> Result<()> {
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())?;

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

/// Best-effort duration in milliseconds from codec metadata.
///
/// Returns `None` if the container does not provide total frames or sample rate.
fn duration_ms_from_codec_params(codec_params: &CodecParameters) -> Option<u64> {
    let frames = codec_params.n_frames?;
    let rate = codec_params.sample_rate? as u64;
    if rate == 0 {
        return None;
    }
    Some(frames.saturating_mul(1000) / rate)
}
