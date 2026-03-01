//! Playback pipeline wiring: resample + playback + optional reporting/cancel.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use cpal::traits::StreamTrait;

use crate::config::PlaybackConfig;
use crate::{playback, queue, resample};
/// Optional knobs for a single playback session (network sessions use these).
///
/// This lets the pipeline wire in:
/// - pause/resume state
/// - a cancel flag (for "next track" interrupts)
pub struct PlaybackSessionOptions {
    /// Optional paused flag (when true, output is silence and queue is not drained).
    pub paused: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Optional cancel flag to terminate playback early.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Optional counter incremented by output frames produced.
    pub played_frames: Option<Arc<AtomicU64>>,
    /// Optional counters updated on underrun.
    pub underrun_frames: Option<Arc<AtomicU64>>,
    pub underrun_events: Option<Arc<AtomicU64>>,
    /// Optional gauge of current buffered frames.
    pub buffered_frames: Option<Arc<AtomicU64>>,
    /// Optional capacity of the output queue (frames).
    pub buffer_capacity_frames: Option<Arc<AtomicU64>>,
    /// Optional user-facing volume percent (0..100).
    pub volume_percent: Option<Arc<std::sync::atomic::AtomicU8>>,
    /// Optional mute flag.
    pub muted: Option<Arc<std::sync::atomic::AtomicBool>>,
}

struct PlaybackState {
    paused: Option<Arc<std::sync::atomic::AtomicBool>>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    played_frames: Option<Arc<AtomicU64>>,
    underrun_frames: Option<Arc<AtomicU64>>,
    underrun_events: Option<Arc<AtomicU64>>,
    buffered_frames: Option<Arc<AtomicU64>>,
    buffer_capacity_frames: Option<Arc<AtomicU64>>,
    volume_percent: Option<Arc<std::sync::atomic::AtomicU8>>,
    muted: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl PlaybackState {
    /// Build internal playback state from public session options.
    fn new(opts: PlaybackSessionOptions) -> Self {
        Self {
            paused: opts.paused,
            cancel: opts.cancel,
            played_frames: opts.played_frames,
            underrun_frames: opts.underrun_frames,
            underrun_events: opts.underrun_events,
            buffered_frames: opts.buffered_frames,
            buffer_capacity_frames: opts.buffer_capacity_frames,
            volume_percent: opts.volume_percent,
            muted: opts.muted,
        }
    }

    /// Placeholder reporter lifecycle hook.
    fn stop_reporter(self) {}
}

/// Wire up the resampler + output stream and block until playback ends or is cancelled.
///
/// This function owns the stage wiring but delegates decoding to `decode::*`.
///
/// If the source sample rate differs from the output, a resampler stage is inserted.
pub fn play_decoded_source(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    playback: &PlaybackConfig,
    src_spec: symphonia::core::audio::SignalSpec,
    srcq: Arc<queue::SharedAudio>,
    opts: PlaybackSessionOptions,
) -> Result<()> {
    let srcq_for_cancel = srcq.clone();
    let state = PlaybackState::new(opts);

    let dst_rate = stream_config.sample_rate;
    let dstq = if src_spec.rate == dst_rate {
        tracing::info!(rate_hz = dst_rate, "resample skipped");
        srcq.clone()
    } else {
        let out = resample::start_resampler(
            srcq,
            src_spec,
            dst_rate,
            resample::ResampleConfig {
                chunk_frames: playback.chunk_frames,
                buffer_seconds: playback.buffer_seconds,
            },
        )?;
        tracing::info!(rate_hz = dst_rate, "resampling");
        out
    };
    if let Some(cap) = &state.buffer_capacity_frames {
        cap.store(dstq.max_frames() as u64, Ordering::Relaxed);
    }

    let stream = playback::build_output_stream(
        device,
        stream_config,
        config.sample_format(),
        &dstq,
        playback::PlaybackConfig {
            refill_max_frames: playback.refill_max_frames,
            paused: state.paused.clone(),
            played_frames: state.played_frames.clone(),
            underrun_frames: state.underrun_frames.clone(),
            underrun_events: state.underrun_events.clone(),
            buffered_frames: state.buffered_frames.clone(),
            cancel_on_error: state.cancel.clone(),
            volume_percent: state.volume_percent.clone(),
            muted: state.muted.clone(),
        },
    )?;
    stream.play()?;

    if let Some(cancel) = &state.cancel {
        let finished_normally = queue::wait_until_done_and_empty_or_cancel(&dstq, cancel);

        if !finished_normally {
            if let Some(paused) = &state.paused {
                paused.store(true, Ordering::Relaxed);
            }
            srcq_for_cancel.close();
            dstq.close();
        }
    } else {
        queue::wait_until_done_and_empty(&dstq);
    }

    // Stop reporter regardless of normal finish vs cancel, then join it.
    state.stop_reporter();

    thread::sleep(Duration::from_millis(100));
    Ok(())
}
