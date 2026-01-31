use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::DeviceTrait;

use crate::queue::SharedAudio;

pub fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    dstq: Arc<SharedAudio>,
) -> Result<cpal::Stream> {
    match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(device, config, dstq),
        cpal::SampleFormat::I16 => build_stream::<i16>(device, config, dstq),
        cpal::SampleFormat::U16 => build_stream::<u16>(device, config, dstq),
        other => Err(anyhow!("Unsupported sample format: {other:?}")),
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    dstq: Arc<SharedAudio>,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels_out = config.channels as usize;

    let state = Arc::new(Mutex::new(PlaybackState {
        pos: 0,
        src_channels: dstq.channels,
        src: Vec::new(),
    }));

    let err_fn = |err| eprintln!("Stream error: {err}");

    let state_cb = state.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut st = state_cb.lock().unwrap();

            if st.pos >= st.src.len() {
                st.pos = 0;
                st.src.clear();

                const REFILL_MAX_FRAMES: usize = 4096;
                if let Some(v) = dstq.try_pop_up_to_frames(REFILL_MAX_FRAMES) {
                    st.src = v;
                }
            }

            let frames = data.len() / channels_out;

            for frame in 0..frames {
                for ch in 0..channels_out {
                    let sample_f32 = next_sample_mapped_from_vec(&mut *st, channels_out, ch);
                    data[frame * channels_out + ch] =
                        <T as cpal::Sample>::from_sample::<f32>(sample_f32);
                }
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}

struct PlaybackState {
    pos: usize,
    src_channels: usize,
    src: Vec<f32>,
}

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