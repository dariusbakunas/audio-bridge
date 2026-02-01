//! Playback pipeline wiring: resample + playback + optional reporting/cancel.

mod progress;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use cpal::traits::StreamTrait;

use crate::{playback, queue, resample};
use progress::{ProgressReporter, start_progress_reporter};

/// Optional knobs for a single playback session (network sessions use these).
///
/// This lets the pipeline wire in:
/// - pause/resume state
/// - a cancel flag (for "next track" interrupts)
/// - a peer socket for progress reporting
pub(crate) struct PlaybackSessionOptions {
    pub(crate) paused: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) peer_tx: Option<std::net::TcpStream>,
}

struct PlaybackState {
    paused: Option<Arc<std::sync::atomic::AtomicBool>>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    played_frames: Option<Arc<AtomicU64>>,
    reporter: Option<ProgressReporter>,
}

impl PlaybackState {
    fn new(opts: PlaybackSessionOptions) -> Self {
        let played_frames = opts
            .peer_tx
            .as_ref()
            .map(|_| Arc::new(AtomicU64::new(0)));

        let reporter = if let Some(peer_tx) = opts.peer_tx {
            Some(start_progress_reporter(
                peer_tx,
                played_frames.clone().unwrap(),
                opts.paused.clone(),
            ))
        } else {
            None
        };

        Self {
            paused: opts.paused,
            cancel: opts.cancel,
            played_frames,
            reporter,
        }
    }

    fn stop_reporter(self) {
        if let Some(reporter) = self.reporter {
            reporter.stop();
        }
    }
}

/// Wire up the resampler + output stream and block until playback ends or is cancelled.
///
/// This function owns the stage wiring but delegates decoding to `decode::*`.
pub(crate) fn play_decoded_source(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    args: &crate::cli::Args,
    src_spec: symphonia::core::audio::SignalSpec,
    srcq: Arc<queue::SharedAudio>,
    opts: PlaybackSessionOptions,
) -> Result<()> {
    let srcq_for_cancel = srcq.clone();
    let state = PlaybackState::new(opts);

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
    tracing::info!(rate_hz = dst_rate, "resampling");

    let stream = playback::build_output_stream(
        device,
        stream_config,
        config.sample_format(),
        &dstq,
        playback::PlaybackConfig {
            refill_max_frames: args.refill_max_frames,
            paused: state.paused.clone(),
            played_frames: state.played_frames.clone(),
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
