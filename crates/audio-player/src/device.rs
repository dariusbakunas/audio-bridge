//! Output device discovery and selection.
//!
//! Thin wrappers around CPAL for:
//! - listing available output devices
//! - selecting either the default device or a device by substring match

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait};
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Pick a CPAL output device.
///
/// - If `needle` is `Some`, chooses the first output device whose name contains the substring
///   (case-insensitive).
/// - Otherwise, returns the host default output device.
///
/// Returns an error if no matching device exists or if the host reports no output devices.
/// Pick the first output device matching `needle` (case-insensitive), or the default device.
///
/// Returns an error if no suitable device is found.
pub fn pick_device(host: &cpal::Host, needle: Option<&str>) -> Result<cpal::Device> {
    let mut devices: Vec<cpal::Device> = host
        .output_devices()
        .context("No output devices")?
        .collect();

    if let Some(needle) = needle {
        if let Some(d) = devices.drain(..).find(|d| {
            d.description()
                .ok()
                .map(|n| matches_device_name(&n.name(), needle))
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
/// Choose the best output config for a target sample rate (or default if unset).
///
/// Prefers exact sample-rate matches when possible.
pub fn pick_output_config(
    device: &cpal::Device,
    target_rate: Option<u32>,
) -> Result<cpal::SupportedStreamConfig> {
    let ranges: Vec<cpal::SupportedStreamConfigRange> =
        device.supported_output_configs()?.collect();
    if ranges.is_empty() {
        return Err(anyhow!("No supported output configs"));
    }

    let mut best: Option<(bool, u32, u8, cpal::SupportedStreamConfig)> = None;

    for range in ranges {
        let min = range.min_sample_rate();
        let max = range.max_sample_rate();
        let rate = pick_rate_for_range(min, max, target_rate);
        let below = target_rate.map(|t| rate <= t).unwrap_or(true);
        let format_rank = sample_format_rank(range.sample_format());
        let cfg = range.with_sample_rate(rate);
        let candidate = (below, rate, format_rank, cfg);
        let replace = match &best {
            None => true,
            Some((b_below, b_rate, b_rank, _)) => {
                is_better_candidate(below, rate, format_rank, *b_below, *b_rate, *b_rank)
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
/// Prefer a fixed buffer size if the device advertises one.
///
/// Returns `None` when the device only supports the default buffer size.
pub fn pick_buffer_size(config: &cpal::SupportedStreamConfig) -> Option<cpal::BufferSize> {
    match config.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            const MAX_FRAMES: u32 = 16_384;
            let chosen = if *max > MAX_FRAMES {
                if *min > MAX_FRAMES { *min } else { MAX_FRAMES }
            } else {
                *max
            };
            Some(cpal::BufferSize::Fixed(chosen))
        }
        cpal::SupportedBufferSize::Unknown => None,
    }
}

fn pick_rate_for_range(min: u32, max: u32, target_rate: Option<u32>) -> u32 {
    let target = target_rate.unwrap_or(u32::MAX);
    if target_rate.is_some() {
        if target >= min && target <= max {
            target
        } else if target < min {
            min
        } else {
            max
        }
    } else {
        max
    }
}

fn sample_format_rank(format: cpal::SampleFormat) -> u8 {
    match format {
        cpal::SampleFormat::F32 => 0,
        cpal::SampleFormat::I32 => 1,
        cpal::SampleFormat::I16 => 2,
        cpal::SampleFormat::U16 => 3,
        _ => 10,
    }
}

fn is_better_candidate(
    below: bool,
    rate: u32,
    format_rank: u8,
    best_below: bool,
    best_rate: u32,
    best_rank: u8,
) -> bool {
    if below != best_below {
        below && !best_below
    } else if rate != best_rate {
        rate > best_rate
    } else {
        format_rank < best_rank
    }
}

/// Print available output devices to stdout.
///
/// This is intended for CLI UX (`--list-devices`) rather than structured output.
/// Log available output devices for the current host.
pub fn list_devices(host: &cpal::Host) -> Result<()> {
    let devices = host.output_devices().context("No output devices")?;
    for (i, d) in devices.enumerate() {
        println!("#{i}: {}", d.description()?);
    }
    Ok(())
}

#[derive(Clone, Debug)]
/// Lightweight output device metadata for UI/device selection.
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub min_rate: u32,
    pub max_rate: u32,
}

/// Return device metadata for output selection UIs.
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
            if min_rate == u32::MAX {
                min_rate = 0;
            }
            if should_warn_invalid_device(&cache_key) {
                tracing::warn!(
                    device = %name,
                    id = %cache_key,
                    min_rate = min_rate,
                    max_rate = max_rate,
                    "skipping device with invalid sample rate range"
                );
            }
            continue;
        }

        update_cached_rates(&cache_key, min_rate, max_rate);
        let id = device_id_for(&d, &name, min_rate, max_rate);
        out.push(DeviceInfo {
            id,
            name,
            min_rate,
            max_rate,
        });
    }
    Ok(out)
}

fn device_id_for(device: &cpal::Device, name: &str, min_rate: u32, max_rate: u32) -> String {
    if let Ok(id) = device.id() {
        return id.to_string();
    }
    hash_device_id(name, min_rate, max_rate)
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

fn invalid_warned() -> &'static Mutex<HashSet<String>> {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn should_warn_invalid_device(key: &str) -> bool {
    if let Ok(mut warned) = invalid_warned().lock() {
        return warned.insert(key.to_string());
    }
    true
}

fn hash_device_id(name: &str, min_rate: u32, max_rate: u32) -> String {
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

fn matches_device_name(name: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    name.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_device_id_is_deterministic() {
        let first = hash_device_id("Device", 44_100, 96_000);
        let second = hash_device_id("Device", 44_100, 96_000);
        assert_eq!(first, second);
    }

    #[test]
    fn hash_device_id_changes_with_inputs() {
        let base = hash_device_id("Device", 44_100, 96_000);
        let other = hash_device_id("Other", 44_100, 96_000);
        assert_ne!(base, other);
    }

    #[test]
    fn cached_rates_roundtrip() {
        let key = "device-key";
        update_cached_rates(key, 48_000, 96_000);
        let cached = cached_rates(key).unwrap();
        assert_eq!(cached, (48_000, 96_000));
    }

    #[test]
    fn matches_device_name_is_case_insensitive() {
        assert!(matches_device_name("USB DAC", "dac"));
        assert!(matches_device_name("usb dac", "USB"));
        assert!(!matches_device_name("USB DAC", "speaker"));
        assert!(!matches_device_name("USB DAC", ""));
    }

    #[test]
    fn pick_rate_for_range_prefers_target_when_in_range() {
        let rate = pick_rate_for_range(44_100, 96_000, Some(48_000));
        assert_eq!(rate, 48_000);
    }

    #[test]
    fn pick_rate_for_range_clamps_below_min() {
        let rate = pick_rate_for_range(44_100, 96_000, Some(22_050));
        assert_eq!(rate, 44_100);
    }

    #[test]
    fn pick_rate_for_range_clamps_above_max() {
        let rate = pick_rate_for_range(44_100, 96_000, Some(192_000));
        assert_eq!(rate, 96_000);
    }

    #[test]
    fn pick_rate_for_range_defaults_to_max() {
        let rate = pick_rate_for_range(44_100, 96_000, None);
        assert_eq!(rate, 96_000);
    }

    #[test]
    fn is_better_candidate_prefers_below_target() {
        let better = is_better_candidate(true, 48_000, 1, false, 48_000, 1);
        assert!(better);
    }

    #[test]
    fn is_better_candidate_prefers_higher_rate() {
        let better = is_better_candidate(true, 96_000, 2, true, 48_000, 2);
        assert!(better);
    }

    #[test]
    fn is_better_candidate_prefers_lower_rank() {
        let better = is_better_candidate(true, 48_000, 0, true, 48_000, 2);
        assert!(better);
    }
}
