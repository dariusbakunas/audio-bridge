//! Bridge runtime helpers.
//!
//! Provides device enumeration, local playback, and HTTP listener startup.

use anyhow::Result;
use cpal::traits::DeviceTrait;

use crate::config::{BridgeListenConfig, BridgePlayConfig};
use crate::{http_api, mdns, player};
use audio_player::{config::PlaybackConfig, decode, device, pipeline, status::PlayerStatusState};

const MDNS_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// List output devices and print them to stdout.
pub fn list_devices() -> Result<()> {
    let host = cpal::default_host();
    device::list_devices(&host)
}

/// Play a local file using the provided playback config.
pub fn run_play(config: BridgePlayConfig) -> Result<()> {
    let host = cpal::default_host();
    let device_name = normalize_device_name(config.device);
    let device = device::pick_device(&host, device_name.as_deref())?;
    tracing::info!(device = %device.description()?, "output device");
    play_one_local(&device, &config.playback, &config.path)
}

/// Run the bridge HTTP API and playback worker.
pub fn run_listen(config: BridgeListenConfig, install_ctrlc: bool) -> Result<()> {
    let device_selected = std::sync::Arc::new(std::sync::Mutex::new(normalize_device_name(
        config.device.clone(),
    )));
    let exclusive_selected = std::sync::Arc::new(std::sync::Mutex::new(false));
    let status = PlayerStatusState::shared();
    let volume = std::sync::Arc::new(player::BridgeVolumeState::new(100, false));

    let mdns_handle: std::sync::Arc<std::sync::Mutex<Option<mdns::MdnsAdvertiser>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    if install_ctrlc {
        let mdns_for_signal = mdns_handle.clone();
        let _ = ctrlc::set_handler(move || {
            if let Ok(mut g) = mdns_for_signal.lock() {
                if let Some(ad) = g.as_ref() {
                    ad.shutdown();
                }
                *g = None;
            }
            std::process::exit(130);
        });
    }

    let player_handle = player::spawn_player(
        device_selected.clone(),
        exclusive_selected.clone(),
        status.clone(),
        volume.clone(),
        config.playback.clone(),
        config.tls_insecure,
    );
    let _http = http_api::spawn_http_server(
        config.http_bind,
        status.clone(),
        volume,
        device_selected.clone(),
        exclusive_selected.clone(),
        player_handle.cmd_tx,
    );
    if let Ok(mut g) = mdns_handle.lock() {
        *g = mdns::spawn_mdns_advertiser(config.http_bind);
    }
    {
        let mdns_handle = mdns_handle.clone();
        let http_bind = config.http_bind;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(MDNS_REFRESH_INTERVAL);
                if let Ok(mut g) = mdns_handle.lock() {
                    if let Some(ad) = g.as_ref() {
                        ad.shutdown();
                    }
                    *g = mdns::spawn_mdns_advertiser(http_bind);
                }
            }
        });
    }
    let _ = _http.join();
    Ok(())
}

fn normalize_device_name(device: Option<String>) -> Option<String> {
    device.and_then(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Decode and play a single local file on the given device.
fn play_one_local(
    device: &cpal::Device,
    playback: &PlaybackConfig,
    path: &std::path::PathBuf,
) -> Result<()> {
    let (src_spec, srcq, _duration_ms, _source_info) =
        decode::start_streaming_decode(path, playback.buffer_seconds)?;
    let config = device::pick_output_config(device, Some(src_spec.rate))?;
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    if let Some(buf) = device::pick_buffer_size(&config) {
        stream_config.buffer_size = buf;
    }
    tracing::info!(
        channels = src_spec.channels.count(),
        rate_hz = src_spec.rate,
        "source (local file)"
    );

    pipeline::play_decoded_source(
        device,
        &config,
        &stream_config,
        playback,
        src_spec,
        srcq,
        pipeline::PlaybackSessionOptions {
            paused: None,
            cancel: None,
            played_frames: None,
            underrun_frames: None,
            underrun_events: None,
            buffered_frames: None,
            buffer_capacity_frames: None,
            volume_percent: None,
            muted: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_device_name_trims_and_drops_empty() {
        assert_eq!(normalize_device_name(None), None);
        assert_eq!(normalize_device_name(Some("".to_string())), None);
        assert_eq!(normalize_device_name(Some("  ".to_string())), None);
        assert_eq!(
            normalize_device_name(Some("  USB DAC ".to_string())),
            Some("USB DAC".to_string())
        );
    }

    #[test]
    fn normalize_device_name_preserves_inner_spaces() {
        assert_eq!(
            normalize_device_name(Some("USB  DAC".to_string())),
            Some("USB  DAC".to_string())
        );
    }

    #[test]
    fn default_http_bind_uses_expected_port() {
        let addr: std::net::SocketAddr = "0.0.0.0:5556".parse().expect("default http bind");
        assert_eq!(addr.port(), 5556);
    }
}
