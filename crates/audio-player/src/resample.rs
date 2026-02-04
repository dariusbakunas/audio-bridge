//! Streaming resample stage.
//!
//! Uses Rubato to convert decoded interleaved `f32` audio from the source rate
//! to the output device rate. Runs in a background thread and writes into a bounded
//! [`SharedAudio`] queue consumed by the playback stage.

use std::sync::Arc;
use std::thread;

use anyhow::Result;
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    calculate_cutoff, Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SignalSpec;

use crate::queue::{calc_max_buffered_samples, PopStrategy, SharedAudio};

/// Configuration for the streaming resampler stage.
#[derive(Clone, Copy, Debug)]
pub struct ResampleConfig {
    /// Input chunk size in frames used for the steady-state resampling loop.
    ///
    /// Larger values reduce per-call overhead and can improve stability at the cost of latency.
    pub chunk_frames: usize,

    /// Target buffering (seconds) for the resampler output queue.
    ///
    /// This provides headroom to keep the audio callback fed even if the resampler thread
    /// is briefly delayed.
    pub buffer_seconds: f32,
}

/// Start a background resampler thread.
///
/// Reads decoded interleaved `f32` samples from `srcq` (at `src_spec.rate`) and produces
/// interleaved `f32` samples at `dst_rate` into a new [`SharedAudio`] queue.
///
/// ## Threading & shutdown
/// - Spawns one thread.
/// - When `srcq` closes and all buffered input is drained, this stage closes its output queue.
///
/// ## Notes
/// This uses Rubato’s streaming sinc resampler. Quality/CPU trade-offs are governed by
/// the internal sinc parameters.
pub fn start_resampler(
    srcq: Arc<SharedAudio>,
    src_spec: SignalSpec,
    dst_rate: u32,
    cfg: ResampleConfig,
) -> Result<Arc<SharedAudio>> {
    let src_rate = src_spec.rate;
    let channels = src_spec.channels.count();

    let max_buffered_samples = calc_max_buffered_samples(dst_rate, channels, cfg.buffer_seconds);
    let dstq = Arc::new(SharedAudio::new(channels, max_buffered_samples));

    let f_ratio = dst_rate as f64 / src_rate as f64;

    let sinc_len = 128;
    let oversampling_factor = 256;
    let interpolation = SincInterpolationType::Cubic;
    let window = WindowFunction::BlackmanHarris2;
    let f_cutoff = calculate_cutoff(sinc_len, window);

    let params = SincInterpolationParameters {
        sinc_len,
        f_cutoff,
        interpolation,
        oversampling_factor,
        window,
    };

    let chunk_in_frames = cfg.chunk_frames.max(1);

    let dstq_thread = dstq.clone();
    thread::spawn(move || {
        let mut resampler: Box<dyn Resampler<f32>> = match Async::<f32>::new_sinc(
            f_ratio,
            1.1,
            &params,
            chunk_in_frames,
            channels,
            FixedAsync::Input,
        ) {
            Ok(r) => Box::new(r),
            Err(e) => {
                tracing::error!("resampler init error: {e:#}");
                dstq_thread.close();
                return;
            }
        };

        let mut out_interleaved = vec![0.0f32; channels * chunk_in_frames * 3];

        let mut indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            active_channels_mask: None,
            partial_len: None,
        };

        loop {
            let interleaved = match srcq.pop(PopStrategy::BlockingExact { frames: chunk_in_frames }) {
                Some(v) => v,
                None => break,
            };

            let input_adapter =
                match InterleavedSlice::new(&interleaved, channels, chunk_in_frames) {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!("interleaved slice (input) error: {e:#}");
                        break;
                    }
                };

            let out_capacity_frames = out_interleaved.len() / channels;
            let mut output_adapter = match InterleavedSlice::new_mut(
                &mut out_interleaved,
                channels,
                out_capacity_frames,
            ) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("interleaved slice (output) error: {e:#}");
                    break;
                }
            };

            indexing.input_offset = 0;
            indexing.output_offset = 0;
            indexing.partial_len = None;

            let (_nbr_in, nbr_out) = match resampler.process_into_buffer(
                &input_adapter,
                &mut output_adapter,
                Some(&indexing),
            ) {
                Ok(x) => x,
                Err(e) => {
                    tracing::error!("resampler process error: {e:#}");
                    break;
                }
            };

            let produced_samples = nbr_out * channels;
            dstq_thread.push_interleaved_blocking(&out_interleaved[..produced_samples]);
        }

        while let Some(tail) = srcq.pop(PopStrategy::BlockingUpTo { max_frames: chunk_in_frames }) {
            let tail_frames = tail.len() / channels;
            if tail_frames == 0 {
                continue;
            }

            let input_adapter = match InterleavedSlice::new(&tail, channels, tail_frames) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("interleaved slice (tail input) error: {e:#}");
                    break;
                }
            };

            let out_capacity_frames = out_interleaved.len() / channels;
            let mut output_adapter = match InterleavedSlice::new_mut(
                &mut out_interleaved,
                channels,
                out_capacity_frames,
            ) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("interleaved slice (tail output) error: {e:#}");
                    break;
                }
            };

            indexing.input_offset = 0;
            indexing.output_offset = 0;
            indexing.partial_len = Some(tail_frames);

            let (_nbr_in, nbr_out) = match resampler.process_into_buffer(
                &input_adapter,
                &mut output_adapter,
                Some(&indexing),
            ) {
                Ok(x) => x,
                Err(e) => {
                    tracing::error!("resampler tail process error: {e:#}");
                    break;
                }
            };

            let produced_samples = nbr_out * channels;
            if produced_samples > 0 {
                dstq_thread.push_interleaved_blocking(&out_interleaved[..produced_samples]);
            }
        }

        dstq_thread.close();
    });

    Ok(dstq)
}
