//! On-demand track analysis (spectrogram + heuristics).

use std::fs::File;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use rustfft::{FftPlanner, num_complex::Complex};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::DecoderOptions,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

pub struct AnalysisOptions {
    pub max_seconds: f32,
    pub width: usize,
    pub height: usize,
    pub window_size: usize,
    pub high_cutoff_hz: Option<f32>,
}

pub struct AnalysisHeuristics {
    pub rolloff_hz: Option<f32>,
    pub ultrasonic_ratio: Option<f32>,
    pub upper_audible_ratio: Option<f32>,
    pub dynamic_range_db: Option<f32>,
    pub notes: Vec<String>,
}

pub struct AnalysisResult {
    pub width: usize,
    pub height: usize,
    pub sample_rate: u32,
    pub duration_ms: Option<u64>,
    pub data: Vec<u8>,
    pub heuristics: AnalysisHeuristics,
}

pub fn analyze_track(path: &Path, options: AnalysisOptions) -> Result<AnalysisResult> {
    let file = File::open(path).with_context(|| format!("open {:?}", path))?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
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

    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("Unknown sample rate"))?;
    let channels = track
        .codec_params
        .channels
        .ok_or_else(|| anyhow!("Unknown channels"))?
        .count();

    let duration_ms = track.codec_params.n_frames.map(|frames| {
        let rate = sample_rate as u64;
        if rate == 0 {
            0
        } else {
            frames.saturating_mul(1000) / rate
        }
    });

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    let max_samples = if options.max_seconds <= 0.0 {
        None
    } else {
        Some((options.max_seconds.max(1.0) * sample_rate as f32) as usize)
    };

    let window_size = options.window_size.clamp(2048, 8192);
    let hop_size = 512usize;
    let max_frames_est = track.codec_params.n_frames.map(|frames| {
        let frames = frames as usize;
        if frames > window_size {
            (frames - window_size) / hop_size
        } else {
            0
        }
    });
    let desired_cols = options.width.clamp(120, 1024);
    let step = max_frames_est
        .map(|frames| (frames / desired_cols).max(1))
        .unwrap_or(1);

    let mut mono_buffer: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(window_size * 2);
    let mut samples_seen: usize = 0;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(window_size);
    let window: Vec<f32> = (0..window_size)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / (window_size - 1) as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();

    let mut spectrogram: Vec<Vec<f32>> = Vec::new();
    let mut ultrasonic_ratios: Vec<f32> = Vec::new();
    let mut upper_audible_ratios: Vec<f32> = Vec::new();
    let mut rolloff_sum = 0.0f32;
    let mut rolloff_count = 0u32;
    let mut rms_windows: Vec<f32> = Vec::new();
    let mut frame_index = 0usize;
    let nyquist = sample_rate as f32 / 2.0;
    let half = window_size / 2;
    let cutoff_hz = options
        .high_cutoff_hz
        .unwrap_or(24_000.0)
        .clamp(1_000.0, nyquist);
    let bin_for_hz = |hz: f32| -> usize {
        if nyquist <= 0.0 {
            0
        } else {
            ((hz / nyquist) * half as f32).ceil() as usize
        }
    };

    while max_samples.map(|max| samples_seen < max).unwrap_or(true) {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };

        let mut sample_buf = SampleBuffer::<f32>::new(decoded.frames() as u64, *decoded.spec());
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        for frame in samples.chunks(channels) {
            let mut sum = 0.0f32;
            for value in frame {
                sum += *value;
            }
            mono_buffer.push_back(sum / channels as f32);
            samples_seen += 1;
            if let Some(max) = max_samples {
                if samples_seen >= max {
                    break;
                }
            }
        }

        while mono_buffer.len() >= window_size {
            if frame_index % step == 0 {
                let mut buffer: Vec<Complex<f32>> = mono_buffer
                    .iter()
                    .take(window_size)
                    .zip(window.iter())
                    .map(|(sample, w)| Complex::new(sample * w, 0.0))
                    .collect();
                fft.process(&mut buffer);

                let mut magnitudes: Vec<f32> = Vec::with_capacity(half);
                let mut energy = 0.0f32;
                for bin in 0..half {
                    let mag = buffer[bin].norm_sqr();
                    magnitudes.push(mag);
                    energy += mag;
                }
                if energy > 0.0 {
                    let denom_start = bin_for_hz(20.0).max(1).min(half);
                    if denom_start < half {
                        let denom = magnitudes[denom_start..].iter().sum::<f32>();
                        if denom > 0.0 {
                            if cutoff_hz < nyquist {
                                let cutoff_bin = bin_for_hz(cutoff_hz).min(half);
                                if cutoff_bin < half {
                                    let sum = magnitudes[cutoff_bin..].iter().sum::<f32>();
                                    ultrasonic_ratios.push(sum / denom);
                                }
                            }

                            let upper_start = bin_for_hz(20_000.0).min(half);
                            if upper_start < half {
                                let upper_end = bin_for_hz(24_000.0).min(half);
                                let end = upper_end.max(upper_start).min(half);
                                let sum = if end > upper_start {
                                    magnitudes[upper_start..end].iter().sum::<f32>()
                                } else {
                                    0.0
                                };
                                upper_audible_ratios.push(sum / denom);
                            }
                        }
                    }
                }

                if energy > 0.0 {
                    let target = energy * 0.95;
                    let mut cumulative = 0.0f32;
                    let mut rolloff_bin = half - 1;
                    for (idx, mag) in magnitudes.iter().enumerate() {
                        cumulative += *mag;
                        if cumulative >= target {
                            rolloff_bin = idx;
                            break;
                        }
                    }
                    let rolloff_hz = (rolloff_bin as f32 / half as f32) * nyquist;
                    rolloff_sum += rolloff_hz;
                    rolloff_count += 1;
                }

                let rms = (magnitudes.iter().sum::<f32>() / magnitudes.len() as f32).sqrt();
                rms_windows.push(rms);

                let downsampled = downsample_bins(&magnitudes, options.height.clamp(64, 512));
                spectrogram.push(downsampled);
            }

            for _ in 0..hop_size {
                mono_buffer.pop_front();
            }
            frame_index += 1;
        }

        if spectrogram.len() >= desired_cols {
            break;
        }
    }

    if samples_seen < sample_rate as usize / 2 {
        return Err(anyhow!("Not enough audio to analyze"));
    }

    let height = options.height.clamp(64, 512);
    let width = spectrogram.len();
    if width == 0 {
        return Err(anyhow!("No spectrogram frames"));
    }

    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;
    for col in &spectrogram {
        for value in col {
            if *value > 0.0 {
                let log = value.log10();
                min_val = min_val.min(log);
                max_val = max_val.max(log);
            }
        }
    }
    if min_val == f32::MAX || max_val == f32::MIN {
        min_val = 0.0;
        max_val = 1.0;
    }

    let mut data: Vec<u8> = Vec::with_capacity(width * height);
    for row in (0..height).rev() {
        for col in 0..width {
            let value = spectrogram[col][row];
            let log = if value <= 0.0 { min_val } else { value.log10() };
            let normalized = ((log - min_val) / (max_val - min_val + 1e-6)).clamp(0.0, 1.0);
            data.push((normalized * 255.0) as u8);
        }
    }

    let rolloff_hz = if rolloff_count > 0 {
        Some(rolloff_sum / rolloff_count as f32)
    } else {
        None
    };
    let ultrasonic_ratio = median(&mut ultrasonic_ratios);
    let upper_audible_ratio = median(&mut upper_audible_ratios);

    let dynamic_range_db = if !rms_windows.is_empty() {
        rms_windows.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = (rms_windows.len() as f32 * 0.1) as usize;
        let noise = rms_windows.get(idx).copied().unwrap_or(0.0).max(1e-8);
        let signal = rms_windows.last().copied().unwrap_or(0.0).max(1e-8);
        Some(20.0 * (signal / noise).log10())
    } else {
        None
    };

    let mut notes = Vec::new();
    if let (Some(ratio), true) = (ultrasonic_ratio, sample_rate >= 88_200) {
        if ratio < 0.005 {
            notes.push("Very low ultrasonic energy; likely upsampled from <=48kHz source.".to_string());
        }
    }
    if let Some(dr) = dynamic_range_db {
        if dr < 90.0 {
            notes.push("Dynamic range suggests <=16-bit source material.".to_string());
        }
    }

    Ok(AnalysisResult {
        width,
        height,
        sample_rate,
        duration_ms: duration_ms.or_else(|| {
            if samples_seen == 0 {
                None
            } else {
                Some(((samples_seen as f32 / sample_rate as f32) * 1000.0) as u64)
            }
        }),
        data,
        heuristics: AnalysisHeuristics {
            rolloff_hz,
            ultrasonic_ratio,
            upper_audible_ratio,
            dynamic_range_db,
            notes,
        },
    })
}

fn downsample_bins(input: &[f32], height: usize) -> Vec<f32> {
    if height == 0 {
        return Vec::new();
    }
    let bins = input.len();
    if bins == height {
        return input.to_vec();
    }
    let mut out = vec![0.0f32; height];
    for i in 0..height {
        let start = (i as f32 / height as f32 * bins as f32).floor() as usize;
        let end = (((i + 1) as f32 / height as f32) * bins as f32).ceil() as usize;
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for idx in start..end.min(bins) {
            sum += input[idx];
            count += 1;
        }
        out[i] = if count > 0 { sum / count as f32 } else { 0.0 };
    }
    out
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        let lo = values[mid - 1];
        let hi = values[mid];
        Some((lo + hi) / 2.0)
    }
}
