use std::collections::VecDeque;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    calculate_cutoff, Async, FixedAsync, Indexing, Resampler,
    SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use symphonia::core::{
    audio::SignalSpec,
    codecs::DecoderOptions,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::core::audio::{SampleBuffer, Signal};

#[derive(Parser, Debug)]
struct Args {
    /// Path to audio file (FLAC recommended)
    path: PathBuf,

    /// List output devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Use a specific output device by substring match
    #[arg(long)]
    device: Option<String>,
}

struct SharedAudio {
    channels: usize,
    queue: Mutex<VecDeque<f32>>,
    not_empty: Condvar,
    not_full: Condvar,
    done: AtomicBool,
    max_buffered_samples: usize,
}

impl SharedAudio {
    fn push_interleaved_blocking(&self, samples: &[f32]) {
        let mut offset = 0;
        while offset < samples.len() {
            let mut q = self.queue.lock().unwrap();

            while q.len() >= self.max_buffered_samples {
                q = self.not_full.wait(q).unwrap();
            }

            let mut pushed_any = false;
            while offset < samples.len() && q.len() < self.max_buffered_samples {
                q.push_back(samples[offset]);
                offset += 1;
                pushed_any = true;
            }

            drop(q);
            if pushed_any {
                self.not_empty.notify_one();
            }
        }
    }

    fn pop_interleaved_frames_blocking(&self, frames: usize) -> Option<Vec<f32>> {
        let want = frames * self.channels;
        let mut q = self.queue.lock().unwrap();

        while q.len() < want && !self.done.load(Ordering::Relaxed) {
            q = self.not_empty.wait(q).unwrap();
        }

        if q.len() < want {
            return None;
        }

        let mut out = Vec::with_capacity(want);
        for _ in 0..want {
            out.push(q.pop_front().unwrap_or(0.0));
        }

        drop(q);
        self.not_full.notify_one();
        Some(out)
    }

    /// Non-blocking: pop up to `max_frames` frames (interleaved).
    /// Returns:
    /// - `Some(vec)` if any frames were available
    /// - `None` if currently empty (regardless of `done`)
    fn try_pop_up_to_frames(&self, max_frames: usize) -> Option<Vec<f32>> {
        let mut q = self.queue.lock().unwrap();

        let available_frames = q.len() / self.channels;
        let take_frames = available_frames.min(max_frames);
        let take_samples = take_frames * self.channels;

        if take_samples == 0 {
            return None;
        }

        let mut out = Vec::with_capacity(take_samples);
        for _ in 0..take_samples {
            out.push(q.pop_front().unwrap_or(0.0));
        }

        drop(q);

        // Space freed for producers.
        self.not_full.notify_one();

        // Also notify waiters that queue state changed (used by wait_until_done_and_empty()).
        self.not_empty.notify_one();

        Some(out)
    }

    /// Convenience helper for “can we shut down?”
    fn is_done_and_empty(&self) -> bool {
        self.done.load(Ordering::Relaxed) && self.queue.lock().unwrap().is_empty()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    let host = cpal::default_host();

    if args.list_devices {
        list_devices(&host)?;
        return Ok(());
    }

    let device = pick_device(&host, args.device.as_deref())?;
    eprintln!("Output device: {}", device.description()?);

    let config = device.default_output_config()?;
    eprintln!("Device default config: {:?}", config);
    let stream_config: cpal::StreamConfig = config.clone().into();

    let (src_spec, srcq) = start_streaming_decode(&args.path)?;
    eprintln!(
        "Source: {}ch @ {} Hz (streaming decode)",
        src_spec.channels.count(),
        src_spec.rate
    );

    let dst_rate = stream_config.sample_rate;
    let dstq = start_resampler(srcq.clone(), src_spec, dst_rate)?;
    eprintln!("Resampling to {} Hz", dst_rate);

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, dstq.clone())?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, dstq.clone())?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, dstq.clone())?,
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    };

    stream.play()?;

    // Block until resampler is done and output queue drains, without polling-sleep.
    wait_until_done_and_empty(&dstq);

    thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn wait_until_done_and_empty(q: &Arc<SharedAudio>) {
    // We reuse `not_empty` as a general “state changed” notification source.
    // (Producers call notify on push; resampler calls notify_all when it sets done.)
    let mut guard = q.queue.lock().unwrap();

    loop {
        let done = q.done.load(Ordering::Relaxed);
        let empty = guard.is_empty();
        if done && empty {
            break;
        }

        // Wait until something changes (or timeout as a safety net).
        let (g, _timeout) = q
            .not_empty
            .wait_timeout(guard, Duration::from_millis(50))
            .unwrap();
        guard = g;
    }
}

fn start_streaming_decode(path: &PathBuf) -> Result<(SignalSpec, Arc<SharedAudio>)> {
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

    let shared = Arc::new(SharedAudio {
        channels,
        queue: Mutex::new(VecDeque::new()),
        not_empty: Condvar::new(),
        not_full: Condvar::new(),
        done: AtomicBool::new(false),
        max_buffered_samples,
    });

    let path_for_thread = path.clone();
    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        if let Err(e) = decode_thread_main(&path_for_thread, &shared_for_thread) {
            eprintln!("Decoder thread error: {e:#}");
        }
        shared_for_thread.done.store(true, Ordering::Relaxed);
        shared_for_thread.not_empty.notify_all();
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

fn start_resampler(
    srcq: Arc<SharedAudio>,
    src_spec: SignalSpec,
    dst_rate: u32,
) -> Result<Arc<SharedAudio>> {
    let src_rate = src_spec.rate;
    let channels = src_spec.channels.count();

    let max_buffered_samples = (dst_rate as usize)
        .saturating_mul(channels)
        .saturating_mul(2);

    let dstq = Arc::new(SharedAudio {
        channels,
        queue: Mutex::new(VecDeque::new()),
        not_empty: Condvar::new(),
        not_full: Condvar::new(),
        done: AtomicBool::new(false),
        max_buffered_samples,
    });

    let f_ratio = dst_rate as f64 / src_rate as f64;

    // Sinc params (reasonable defaults; tweak if you want lower CPU/higher quality).
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

    // Fixed input chunk size (in frames) for streaming.
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
                dstq_thread.done.store(true, Ordering::Relaxed);
                dstq_thread.not_empty.notify_all();
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
                None => break, // source done/drained (might still have a tail < chunk)
            };

            let input_adapter = match InterleavedSlice::new(&interleaved, channels, chunk_in_frames) {
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

        // Drain tail (partial chunk) if any.
        // Keep pulling until the decode thread says "done" and the queue is empty.
        loop {
            let tail = match srcq.try_pop_up_to_frames(chunk_in_frames) {
                Some(v) => v,
                None => {
                    if srcq.is_done_and_empty() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
            };

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

        dstq_thread.done.store(true, Ordering::Relaxed);
        dstq_thread.not_empty.notify_all();
    });

    Ok(dstq)
}

// build_stream now takes SharedAudio instead of Vec<f32>
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
                // else: underrun → we’ll output zeros below.
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

fn pick_device(host: &cpal::Host, needle: Option<&str>) -> Result<cpal::Device> {
    let mut devices: Vec<cpal::Device> = host
        .output_devices()
        .context("No output devices")?
        .collect();

    if let Some(needle) = needle {
        let needle_lc = needle.to_lowercase();
        if let Some(d) = devices
            .drain(..)
            .find(|d| d.description().ok().map(|n| n.name().to_lowercase().contains(&needle_lc)).unwrap_or(false))
        {
            return Ok(d);
        }
        return Err(anyhow!("No output device matched: {needle}"));
    }

    host.default_output_device()
        .ok_or_else(|| anyhow!("No default output device"))
}

fn list_devices(host: &cpal::Host) -> Result<()> {
    let devices = host.output_devices().context("No output devices")?;
    for (i, d) in devices.enumerate() {
        eprintln!("#{i}: {}", d.description()?);
    }
    Ok(())
}

// CPAL callback now just drains already-resampled interleaved f32.
struct PlaybackState {
    pos: usize,
    src_channels: usize,
    src: Vec<f32>,
}
