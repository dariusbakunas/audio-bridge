use std::collections::VecDeque;
use std::fs::File;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use symphonia::core::{
    audio::{SignalSpec},
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
    src_channels: usize,

    // Interleaved f32 samples (src_channels per frame).
    queue: Mutex<VecDeque<f32>>,

    // Decoder thread waits when queue is "too full".
    not_full: Condvar,

    // Signals end-of-file / decode completion.
    done: AtomicBool,

    // Bounded buffering (in samples, not frames).
    max_buffered_samples: usize,
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

    // Open output stream using device default config.
    // NOTE: this does NOT resample; if file sample-rate != device sample-rate, playback speed/pitch will be off.
    let config = device.default_output_config()?;
    eprintln!("Device default config: {:?}", config);

    // Initialize decoder once to discover source spec (rate/channels), then start decode thread.
    let (src_spec, shared) = start_streaming_decode(&args.path)?;
    eprintln!(
        "Source: {}ch @ {} Hz (streaming decode)",
        src_spec.channels.count(),
        src_spec.rate
    );

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config.into(), shared.clone())?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config.into(), shared.clone())?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config.into(), shared.clone())?,
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    };

    stream.play()?;

    // Block until decode is done and buffer drains.
    loop {
        let done = shared.done.load(Ordering::Relaxed);
        let buffered = shared.queue.lock().unwrap().len();
        if done && buffered == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }

    // Give the device a tiny moment to flush last callback(s).
    thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn start_streaming_decode(path: &PathBuf) -> Result<(SignalSpec, Arc<SharedAudio>)> {
    // Open once to discover spec & channel count.
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

    let src_channels = track
        .codec_params
        .channels
        .ok_or_else(|| anyhow!("Unknown channels"))?
        .count();

    let src_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;

    let src_spec = SignalSpec::new(src_rate, track.codec_params.channels.unwrap());

    // Buffer about ~2 seconds worth of audio by default.
    let max_buffered_samples = (src_rate as usize)
        .saturating_mul(src_channels)
        .saturating_mul(2);

    let shared = Arc::new(SharedAudio {
        src_channels,
        queue: Mutex::new(VecDeque::new()),
        not_full: Condvar::new(),
        done: AtomicBool::new(false),
        max_buffered_samples,
    });

    // Now start a dedicated decode thread that re-opens and decodes from the beginning.
    // (This keeps lifetimes simple and avoids moving `format` out of this function.)
    let path_for_thread = path.clone();
    let shared_for_thread = shared.clone();

    thread::spawn(move || {
        if let Err(e) = decode_thread_main(&path_for_thread, &shared_for_thread) {
            eprintln!("Decoder thread error: {e:#}");
        }
        shared_for_thread.done.store(true, Ordering::Relaxed);
    });

    Ok((src_spec, shared))
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
        let samples = sample_buf.samples();

        // Push into bounded queue; if it's "full", wait (decoder thread only).
        let mut offset = 0;
        while offset < samples.len() {
            let mut q = shared.queue.lock().unwrap();

            // Wait until there is space for at least 1 sample.
            while q.len() >= shared.max_buffered_samples {
                q = shared.not_full.wait(q).unwrap();
            }

            // Push as many samples as we can without exceeding the bound too much.
            // (Simple approach: push until full; loop continues if more remain.)
            while offset < samples.len() && q.len() < shared.max_buffered_samples {
                q.push_back(samples[offset]);
                offset += 1;
            }
        }
    }

    Ok(())
}

fn samples_len_frames(spec: &SignalSpec, samples: &[f32]) -> usize {
    let ch = spec.channels.count();
    samples.len() / ch
}

// build_stream now takes SharedAudio instead of Vec<f32>
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<SharedAudio>,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels_out = config.channels as usize;

    let state = Arc::new(Mutex::new(PlaybackState {
        src_channels: shared.src_channels,
        shared,
    }));

    let err_fn = |err| eprintln!("Stream error: {err}");

    let state_cb = state.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut st = state_cb.lock().unwrap();

            let frames = data.len() / channels_out;
            for frame in 0..frames {
                for ch in 0..channels_out {
                    let sample_f32 = st.next_sample_mapped(channels_out, ch);
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

fn decode_to_f32_interleaved(path: &PathBuf) -> Result<(SignalSpec, Vec<f32>)> {
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

    let mut decoder = symphonia::default::get_codecs().make(
        &track.codec_params,
        &DecoderOptions::default(),
    )?;

    let spec = track
        .codec_params
        .sample_rate
        .map(|rate| SignalSpec::new(rate, track.codec_params.channels.ok_or_else(|| anyhow!("Unknown channels")).expect("Unknown channels")))
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;

    let mut out: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break, // EOF
        };

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut sample_buf =
            SampleBuffer::<f32>::new(decoded.frames() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);
        out.extend_from_slice(sample_buf.samples());
    }

    Ok((spec, out))
}

// Replaces your old Vec-backed PlaybackState.
// The CPAL callback pulls samples from SharedAudio::queue.
struct PlaybackState {
    src_channels: usize,
    shared: Arc<SharedAudio>,
}

impl PlaybackState {
    fn next_sample_mapped(&mut self, dst_channels: usize, dst_ch: usize) -> f32 {
        // Fast path: grab a whole source frame (src_channels samples) if available.
        let mut frame: [f32; 2] = [0.0, 0.0]; // enough for mono/stereo; other layouts handled fallback below

        let got_frame = {
            let mut q = self.shared.queue.lock().unwrap();

            if q.len() >= self.src_channels {
                if self.src_channels >= 1 {
                    frame[0] = q.pop_front().unwrap_or(0.0);
                }
                if self.src_channels >= 2 {
                    frame[1] = q.pop_front().unwrap_or(0.0);
                } else if self.src_channels == 1 {
                    // keep frame[1] as 0.0
                } else {
                    // src_channels == 0 (shouldn't happen), treat as silence.
                }

                // Notify decoder thread that space freed up.
                self.shared.not_full.notify_one();
                true
            } else {
                false
            }
        };

        if !got_frame {
            return 0.0;
        }

        let get_src = |ch: usize| -> f32 {
            match ch {
                0 => frame[0],
                1 => frame[1],
                _ => 0.0,
            }
        };

        match (self.src_channels, dst_channels) {
            (1, 1) => get_src(0),
            (2, 2) => get_src(dst_ch.min(1)),
            (2, 1) => 0.5 * (get_src(0) + get_src(1)),
            (1, 2) => get_src(0),
            _ => get_src(dst_ch.min(self.src_channels.saturating_sub(1))),
        }
    }
}