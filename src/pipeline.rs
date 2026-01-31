//! Playback pipeline wiring: resample + playback + optional reporting/cancel.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use cpal::traits::StreamTrait;
use crate::{playback, queue, resample};

pub(crate) struct PlaybackSessionOptions {
    pub(crate) paused: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) peer_tx: Option<std::net::TcpStream>,
}

struct ProgressReporter {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl ProgressReporter {
    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

fn start_progress_reporter(
    mut peer_tx: std::net::TcpStream,
    played_frames: Arc<AtomicU64>,
    paused: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> ProgressReporter {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_thread = stop.clone();

    peer_tx.set_nodelay(true).ok();
    let handle = thread::spawn(move || loop {
        if stop_thread.load(Ordering::Relaxed) {
            break;
        }

        let frames = played_frames.load(Ordering::Relaxed);
        let is_paused = paused
            .as_ref()
            .map(|p| p.load(Ordering::Relaxed))
            .unwrap_or(false);

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

        thread::sleep(Duration::from_millis(200));
    });

    ProgressReporter { stop, handle }
}

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

    let stream = playback::build_output_stream(
        device,
        stream_config,
        config.sample_format(),
        &dstq,
        playback::PlaybackConfig {
            refill_max_frames: args.refill_max_frames,
            paused: opts.paused.clone(),
            played_frames,
        },
    )?;
    stream.play()?;

    if let Some(cancel) = opts.cancel {
        let finished_normally = queue::wait_until_done_and_empty_or_cancel(&dstq, &cancel);

        if !finished_normally {
            if let Some(paused) = &opts.paused {
                paused.store(true, Ordering::Relaxed);
            }
            srcq_for_cancel.close();
            dstq.close();
        }
    } else {
        queue::wait_until_done_and_empty(&dstq);
    }

    // Stop reporter regardless of normal finish vs cancel, then join it.
    if let Some(reporter) = reporter {
        reporter.stop();
    }

    thread::sleep(Duration::from_millis(100));
    Ok(())
}
