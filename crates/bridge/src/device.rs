//! Output device discovery and selection.
//!
//! Thin wrappers around CPAL for:
//! - listing available output devices
//! - selecting either the default device or a device by substring match

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};

/// Select an output device and its default output config.
///
/// - If `needle` is `Some`, chooses the first output device whose name contains the substring
///   (case-insensitive).
/// - Otherwise, returns the host default output device.
pub fn select_output(
    host: &cpal::Host,
    needle: Option<&str>,
) -> Result<(cpal::Device, cpal::SupportedStreamConfig)> {
    let device = pick_device(host, needle)?;
    let config = device.default_output_config()?;
    Ok((device, config))
}

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

/// Return available output device names.
pub fn list_device_names(host: &cpal::Host) -> Result<Vec<String>> {
    let devices = host.output_devices().context("No output devices")?;
    let mut out = Vec::new();
    for d in devices {
        out.push(d.description()?.to_string());
    }
    Ok(out)
}

/// Return the current default output device name, if any.
pub fn default_device_name() -> Option<String> {
    let host = cpal::default_host();
    host.default_output_device()
        .and_then(|d| d.description().ok().map(|desc| desc.to_string()))
}
