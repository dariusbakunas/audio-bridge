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

use crate::queue::{SharedAudio, calc_max_buffered_samples};
use anyhow::{Context, Result, anyhow};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::io::MediaSource;
use symphonia::core::{
    audio::SignalSpec, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
    meta::MetadataOptions, probe::Hint,
};

/// Metadata captured while probing the source.
#[derive(Clone, Debug, Default)]
pub struct SourceInfo {
    /// Codec name (best-effort).
    pub codec: Option<String>,
    /// Source bit depth (best-effort).
    pub bit_depth: Option<u16>,
    /// Container/extension hint (best-effort).
    pub container: Option<String>,
}

/// Start decoding from an arbitrary Symphonia [`MediaSource`] (seekable or not).
///
/// This is the shared entry point used by both:
/// - local file playback
/// - network-spooled playback
///
/// The queue is closed on EOF or error.
pub fn start_streaming_decode_from_media_source(
    source: Box<dyn MediaSource>,
    hint: Hint,
    buffer_seconds: f32,
) -> Result<(SignalSpec, Arc<SharedAudio>, Option<u64>, SourceInfo)> {
    start_streaming_decode_from_media_source_at(source, hint, buffer_seconds, None)
}

/// Start decoding from a [`MediaSource`] at a requested offset (milliseconds).
///
/// Returns the stream spec, queue, optional duration, and captured source metadata.
pub fn start_streaming_decode_from_media_source_at(
    source: Box<dyn MediaSource>,
    hint: Hint,
    buffer_seconds: f32,
    seek_ms: Option<u64>,
) -> Result<(SignalSpec, Arc<SharedAudio>, Option<u64>, SourceInfo)> {
    // Probe once to get spec.
    let mss = MediaSourceStream::new(source, Default::default());

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;
    if let Some(ms) = seek_ms {
        if ms > 0 {
            let secs = ms / 1000;
            let frac = (ms % 1000) as f64 / 1000.0;
            let time = symphonia::core::units::Time::new(secs, frac);
            let _ = format.seek(
                symphonia::core::formats::SeekMode::Accurate,
                symphonia::core::formats::SeekTo::Time {
                    time,
                    track_id: None,
                },
            );
        }
    }

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
    let source_info = SourceInfo {
        codec: codec_name_from_params(&codec_params),
        bit_depth: codec_params
            .bits_per_sample
            .or(codec_params.bits_per_coded_sample)
            .and_then(|v| u16::try_from(v).ok()),
        container: None,
    };

    let max_buffered_samples = calc_max_buffered_samples(rate, channels, buffer_seconds);
    let shared = Arc::new(SharedAudio::new(channels, max_buffered_samples));

    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        if let Err(e) = decode_format_loop(format, codec_params, &shared_for_thread) {
            tracing::error!("decoder thread error: {e:#}");
        }
        shared_for_thread.close();
    });

    Ok((spec, shared, duration_ms, source_info))
}

/// Start a background decoder thread that streams interleaved `f32` samples from `path`.
///
/// Returns the stream spec, queue, optional duration, and captured source metadata.
pub fn start_streaming_decode(
    path: &PathBuf,
    buffer_seconds: f32,
) -> Result<(SignalSpec, Arc<SharedAudio>, Option<u64>, SourceInfo)> {
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
    let mut decoder =
        symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;

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

/// Best-effort codec label used for status payloads.
fn codec_name_from_params(params: &CodecParameters) -> Option<String> {
    use symphonia::core::codecs::*;
    let name = match params.codec {
        CODEC_TYPE_FLAC => "FLAC",
        CODEC_TYPE_MP3 => "MP3",
        CODEC_TYPE_AAC => "AAC",
        CODEC_TYPE_ALAC => "ALAC",
        CODEC_TYPE_VORBIS => "VORBIS",
        CODEC_TYPE_OPUS => "OPUS",
        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE => "PCM_S16",
        CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE => "PCM_S24",
        CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE => "PCM_S32",
        CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE => "PCM_F32",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::codecs::*;

    #[test]
    fn duration_ms_from_codec_params_handles_zero_rate() {
        let mut params = CodecParameters::new();
        params.sample_rate = Some(0);
        params.n_frames = Some(100);
        assert!(duration_ms_from_codec_params(&params).is_none());
    }

    #[test]
    fn duration_ms_from_codec_params_computes() {
        let mut params = CodecParameters::new();
        params.sample_rate = Some(48_000);
        params.n_frames = Some(96_000);
        let ms = duration_ms_from_codec_params(&params).unwrap();
        assert_eq!(ms, 2000);
    }

    #[test]
    fn codec_name_from_params_maps_known_codecs() {
        let mut params = CodecParameters::new();
        params.codec = CODEC_TYPE_FLAC;
        assert_eq!(codec_name_from_params(&params), Some("FLAC".to_string()));
        params.codec = CODEC_TYPE_PCM_S16LE;
        assert_eq!(codec_name_from_params(&params), Some("PCM_S16".to_string()));
    }

    #[test]
    fn codec_name_from_params_unknown_returns_none() {
        let params = CodecParameters::new();
        assert!(codec_name_from_params(&params).is_none());
    }
}
