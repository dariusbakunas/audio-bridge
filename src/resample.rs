use std::sync::Arc;
use std::thread;

use anyhow::Result;
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    calculate_cutoff, Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SignalSpec;

use crate::queue::SharedAudio;

pub fn start_resampler(
    srcq: Arc<SharedAudio>,
    src_spec: SignalSpec,
    dst_rate: u32,
) -> Result<Arc<SharedAudio>> {
    let src_rate = src_spec.rate;
    let channels = src_spec.channels.count();

    let max_buffered_samples = (dst_rate as usize)
        .saturating_mul(channels)
        .saturating_mul(2);

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

    let chunk_in_frames = 1024usize;

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
                eprintln!("Resampler init error: {e:#}");
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
            let interleaved = match srcq.pop_interleaved_frames_blocking(chunk_in_frames) {
                Some(v) => v,
                None => break,
            };

            let input_adapter =
                match InterleavedSlice::new(&interleaved, channels, chunk_in_frames) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("InterleavedSlice(input) error: {e:#}");
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
                    eprintln!("InterleavedSlice(output) error: {e:#}");
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
                    eprintln!("Resampler process error: {e:#}");
                    break;
                }
            };

            let produced_samples = nbr_out * channels;
            dstq_thread.push_interleaved_blocking(&out_interleaved[..produced_samples]);
        }

        while let Some(tail) = srcq.pop_up_to_frames_blocking(chunk_in_frames) {
            let tail_frames = tail.len() / channels;
            if tail_frames == 0 {
                continue;
            }

            let input_adapter = match InterleavedSlice::new(&tail, channels, tail_frames) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("InterleavedSlice(tail input) error: {e:#}");
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
                    eprintln!("InterleavedSlice(tail output) error: {e:#}");
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
                    eprintln!("Resampler tail process error: {e:#}");
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