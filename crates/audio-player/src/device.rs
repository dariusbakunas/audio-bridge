//! Output device discovery and selection.
//!
//! Thin wrappers around CPAL for:
//! - listing available output devices
//! - selecting either the default device or a device by substring match

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::{Mutex, OnceLock};

/// Pick a CPAL output device.
///
/// - If `needle` is `Some`, chooses the first output device whose name contains the substring
///   (case-insensitive).
/// - Otherwise, returns the host default output device.
///
/// Returns an error if no matching device exists or if the host reports no output devices.
pub fn pick_device(host: &cpal::Host, needle: Option<&str>) -> Result<cpal::Device> {
    let mut devices: Vec<cpal::Device> = host
        .output_devices()
        .context("No output devices")?
        .collect();

    if let Some(needle) = needle {
        let needle_lc = needle.to_lowercase();
        if let Some(d) = devices.drain(..).find(|d| {
            d.description()
                .ok()
                .map(|n| n.name().to_lowercase().contains(&needle_lc))
                .unwrap_or(false)
        }) {
            return Ok(d);
        }
        return Err(anyhow!("No output device matched: {needle}"));
    }

    host.default_output_device()
        .ok_or_else(|| anyhow!("No default output device"))
}

/// Pick the best supported output config for the device.
///
/// If `target_rate` is `Some`, prefer the highest supported sample rate that is
/// **<= target_rate**; if none are <=, choose the lowest supported rate above it.
/// If `None`, choose the highest supported rate.
pub fn pick_output_config(
    device: &cpal::Device,
    target_rate: Option<u32>,
) -> Result<cpal::SupportedStreamConfig> {
    let ranges: Vec<cpal::SupportedStreamConfigRange> =
        device.supported_output_configs()?.collect();
    if ranges.is_empty() {
        return Err(anyhow!("No supported output configs"));
    }

    let target = target_rate.unwrap_or(u32::MAX);
    let mut best: Option<(bool, u32, u8, cpal::SupportedStreamConfig)> = None;

    for range in ranges {
        let min = range.min_sample_rate();
        let max = range.max_sample_rate();
        let rate = if target_rate.is_some() {
            if target >= min && target <= max {
                target
            } else if target < min {
                min
            } else {
                max
            }
        } else {
            max
        };
        let below = target_rate.map(|t| rate <= t).unwrap_or(true);
        let format_rank = match range.sample_format() {
            cpal::SampleFormat::F32 => 0,
            cpal::SampleFormat::I32 => 1,
            cpal::SampleFormat::I16 => 2,
            cpal::SampleFormat::U16 => 3,
            _ => 10,
        };
        let cfg = range.with_sample_rate(rate);
        let candidate = (below, rate, format_rank, cfg);
        let replace = match &best {
            None => true,
            Some((b_below, b_rate, b_rank, _)) => {
                if below != *b_below {
                    below && !*b_below
                } else if rate != *b_rate {
                    rate > *b_rate
                } else {
                    format_rank < *b_rank
                }
            }
        };
        if replace {
            best = Some(candidate);
        }
    }

    Ok(best.unwrap().3)
}

/// Pick a stream buffer size, preferring larger values to reduce underruns.
///
/// If the device reports a range, choose the max. If `Unknown`, return `None`
/// so CPAL uses the device default.
pub fn pick_buffer_size(
    config: &cpal::SupportedStreamConfig,
) -> Option<cpal::BufferSize> {
    match config.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            const MAX_FRAMES: u32 = 16_384;
            let chosen = if *max > MAX_FRAMES {
                if *min > MAX_FRAMES {
                    *min
                } else {
                    MAX_FRAMES
                }
            } else {
                *max
            };
            Some(cpal::BufferSize::Fixed(chosen))
        }
        cpal::SupportedBufferSize::Unknown => None,
    }
}

/// Print available output devices to stdout.
///
/// This is intended for CLI UX (`--list-devices`) rather than structured output.
pub fn list_devices(host: &cpal::Host) -> Result<()> {
    let devices = host.output_devices().context("No output devices")?;
    for (i, d) in devices.enumerate() {
        println!("#{i}: {}", d.description()?);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub min_rate: u32,
    pub max_rate: u32,
}

pub fn list_device_infos(host: &cpal::Host) -> Result<Vec<DeviceInfo>> {
    let devices = host.output_devices().context("No output devices")?;
    let mut out = Vec::new();
    for d in devices {
        let name = d.description()?.to_string();
        let cache_key = device_cache_key(&d, &name);
        let mut min_rate = u32::MAX;
        let mut max_rate = 0u32;
        match d.supported_output_configs() {
            Ok(ranges) => {
                for r in ranges {
                    min_rate = min_rate.min(r.min_sample_rate());
                    max_rate = max_rate.max(r.max_sample_rate());
                }
                if min_rate == u32::MAX {
                    min_rate = 0;
                }
            }
            Err(_) => {}
        }

        if min_rate == 0 || max_rate == 0 || max_rate < min_rate {
            if let Ok(default_cfg) = d.default_output_config() {
                let sr = default_cfg.sample_rate();
                min_rate = sr;
                max_rate = sr;
            }
        }

        if min_rate == 0 || max_rate == 0 || max_rate < min_rate {
            if let Some((cached_min, cached_max)) = cached_rates(&cache_key) {
                min_rate = cached_min;
                max_rate = cached_max;
            }
        }

        if min_rate == 0 || max_rate == 0 || max_rate < min_rate {
            tracing::warn!(
                device = %name,
                id = %cache_key,
                min_rate = min_rate,
                max_rate = max_rate,
                "skipping device with invalid sample rate range"
            );
            continue;
        }

        update_cached_rates(&cache_key, min_rate, max_rate);
        let id = device_id_for(&d, &name, min_rate, max_rate);
        out.push(DeviceInfo { id, name, min_rate, max_rate });
    }
    Ok(out)
}

fn device_id_for(device: &cpal::Device, name: &str, min_rate: u32, max_rate: u32) -> String {
    if let Ok(id) = device.id() {
        return id.to_string();
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut input = String::new();
    input.push_str(name);
    input.push('|');
    input.push_str(&min_rate.to_string());
    input.push('|');
    input.push_str(&max_rate.to_string());
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn device_cache_key(device: &cpal::Device, name: &str) -> String {
    if let Ok(id) = device.id() {
        return id.to_string();
    }
    name.to_string()
}

fn rates_cache() -> &'static Mutex<std::collections::HashMap<String, (u32, u32)>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, (u32, u32)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn cached_rates(key: &str) -> Option<(u32, u32)> {
    rates_cache().lock().ok().and_then(|m| m.get(key).copied())
}

fn update_cached_rates(key: &str, min_rate: u32, max_rate: u32) {
    if let Ok(mut m) = rates_cache().lock() {
        m.insert(key.to_string(), (min_rate, max_rate));
    }
}
