//! Playback stage (CPAL output stream).
//!
//! Builds the CPAL output stream and provides the real-time audio callback.
//! The callback:
//! - refills a small local buffer from the shared queue without blocking
//! - applies basic channel mapping (mono↔stereo, best-effort otherwise)
//! - converts `f32` samples to the device sample format

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use anyhow::{anyhow, Result};
use cpal::traits::DeviceTrait;

use crate::queue::{PopStrategy, SharedAudio};

/// Configuration for the playback stage (CPAL output callback).
#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    /// Maximum number of frames to pull from the queue per refill.
    ///
    /// Larger values reduce mutex/queue churn but can increase latency.
    pub refill_max_frames: usize,

    /// When set and `true`, the callback outputs silence and **does not drain** the queue.
    ///
    /// This implements “pause means pause” (no skipping ahead).
    pub paused: Option<Arc<AtomicBool>>,

    /// When set, the callback increments this by the number of output frames produced.
    pub played_frames: Option<Arc<AtomicU64>>,

    /// When set, the callback increments these when it has to output silence.
    pub underrun_frames: Option<Arc<AtomicU64>>,
    pub underrun_events: Option<Arc<AtomicU64>>,
}

/// Build a CPAL output stream that plays audio from `dstq`.
///
/// `dstq` must contain **interleaved `f32` samples** already converted to the device sample rate.
/// The callback performs:
/// - a non-blocking refill from the queue
/// - channel mapping (e.g., mono↔stereo)
/// - conversion from `f32` to the device sample format
///
/// ## Real-time constraints
/// The callback never blocks on locks longer than necessary and never waits on a condition variable.
/// Underruns are filled with zeros (silence).
pub fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    dstq: &Arc<SharedAudio>,
    cfg: PlaybackConfig,
) -> Result<cpal::Stream> {
    match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(device, config, dstq, cfg),
        cpal::SampleFormat::I16 => build_stream::<i16>(device, config, dstq, cfg),
        cpal::SampleFormat::I32 => build_stream::<i32>(device, config, dstq, cfg),
        cpal::SampleFormat::U16 => build_stream::<u16>(device, config, dstq, cfg),
        other => Err(anyhow!("Unsupported sample format: {other:?}")),
    }
}

/// Type-specialized stream builder for CPAL sample formats.
///
/// This sets up a callback that drains `dstq` in bursts (up to `refill_max_frames`) and writes
/// samples into the output buffer, applying simple channel mapping.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    dstq: &Arc<SharedAudio>,
    cfg: PlaybackConfig,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels_out = config.channels as usize;

    let state = Arc::new(Mutex::new(PlaybackState {
        pos: 0,
        src_channels: dstq.channels(),
        src: Vec::new(),
    }));

    let refill_max_frames = cfg.refill_max_frames.max(1);
    let dstq_cb = dstq.clone();
    let paused_flag = cfg.paused.clone();
    let played_frames = cfg.played_frames.clone();
    let underrun_frames = cfg.underrun_frames.clone();
    let underrun_events = cfg.underrun_events.clone();

    let err_fn = |err| tracing::warn!("stream error: {err}");

    let state_cb = state.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            if let Some(p) = &paused_flag {
                if p.load(Ordering::Relaxed) {
                    data.fill(<T as cpal::Sample>::from_sample::<f32>(0.0));
                    return;
                }
            }

            let mut st = state_cb.lock().unwrap();

            let frames = data.len() / channels_out;
            let mut filled_frames = 0usize;

            for frame in 0..frames {
                if st.pos >= st.src.len() {
                    st.pos = 0;
                    st.src.clear();
                    if let Some(v) = dstq_cb.pop(PopStrategy::NonBlocking { max_frames: refill_max_frames }) {
                        st.src = v;
                    } else {
                        // No more audio ready; fill the rest with silence.
                        if let Some(events) = &underrun_events {
                            events.fetch_add(1, Ordering::Relaxed);
                        }
                        if let Some(frames_counter) = &underrun_frames {
                            let remaining = frames.saturating_sub(frame);
                            frames_counter.fetch_add(remaining as u64, Ordering::Relaxed);
                        }
                        for idx in (frame * channels_out)..data.len() {
                            data[idx] = <T as cpal::Sample>::from_sample::<f32>(0.0);
                        }
                        break;
                    }
                }
                for ch in 0..channels_out {
                    let sample_f32 = next_sample_mapped_from_vec(&mut *st, channels_out, ch);
                    data[frame * channels_out + ch] =
                        <T as cpal::Sample>::from_sample::<f32>(sample_f32);
                }
                filled_frames += 1;
            }

            if let Some(counter) = &played_frames {
                if filled_frames > 0 {
                    counter.fetch_add(filled_frames as u64, Ordering::Relaxed);
                }
            }
        },
        err_fn,
        None,
    )?;


    Ok(stream)
}

/// Local playback buffer state for the CPAL callback.
///
/// We keep a small Vec of interleaved samples fetched from `SharedAudio` so the callback
/// can run quickly without frequently locking the queue.
struct PlaybackState {
    pos: usize,
    src_channels: usize,
    src: Vec<f32>,
}

/// Fetch the next output sample after applying basic channel mapping.
///
/// Mapping rules:
/// - mono → stereo: duplicate channel 0
/// - stereo → mono: average L/R
/// - stereo → stereo: pass-through
/// - other layouts: best-effort “clamp to available channels”
/// Read one output sample for `dst_ch`, applying a simple channel mapping.
///
/// `st.pos` advances once per destination frame (after the last channel).
fn next_sample_mapped_from_vec(st: &mut PlaybackState, dst_channels: usize, dst_ch: usize) -> f32 {
    if st.pos >= st.src.len() {
        return 0.0;
    }

    let frame_start = st.pos;
    let get_src = |ch: usize, st: &PlaybackState| -> f32 {
        if ch < st.src_channels && frame_start + ch < st.src.len() {
            st.src[frame_start + ch]
        } else {
            0.0
        }
    };

    let out = match (st.src_channels, dst_channels) {
        (1, 1) => get_src(0, st),
        (2, 2) => get_src(dst_ch.min(1), st),
        (2, 1) => 0.5 * (get_src(0, st) + get_src(1, st)),
        (1, 2) => get_src(0, st),
        _ => get_src(dst_ch.min(st.src_channels.saturating_sub(1)), st),
    };

    if dst_ch + 1 == dst_channels {
        st.pos += st.src_channels;
    }
    out
}
