use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};

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

pub fn list_devices(host: &cpal::Host) -> Result<()> {
    let devices = host.output_devices().context("No output devices")?;
    for (i, d) in devices.enumerate() {
        eprintln!("#{i}: {}", d.description()?);
    }
    Ok(())
}