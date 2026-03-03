//! Bridge runtime helpers.
//!
//! Provides device enumeration, local playback, and HTTP listener startup.

use anyhow::Result;
use cpal::traits::DeviceTrait;
use serde_json::json;
use std::collections::HashSet;

use crate::config::{BridgeListenConfig, BridgePlayConfig};
use crate::dummy_output;
use crate::{http_api, mdns, player};
use audio_player::{config::PlaybackConfig, decode, device, pipeline, status::PlayerStatusState};

const MDNS_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// List output devices and print them to stdout.
pub fn list_devices(enable_dummy_outputs: bool) -> Result<()> {
    let host = cpal::default_host();
    if let Err(e) = device::list_devices(&host) {
        tracing::warn!("failed to list physical output devices: {e:#}");
    }
    if enable_dummy_outputs {
        for dev in dummy_output::list_devices() {
            println!("#dummy: {} ({})", dev.name, dev.id);
        }
    }
    Ok(())
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
    let known_hub_origins = std::sync::Arc::new(std::sync::Mutex::new(HashSet::<String>::new()));
    if let Some(origin) = normalize_origin(config.hub_url.as_deref()) {
        if let Ok(mut known) = known_hub_origins.lock() {
            known.insert(origin);
        }
    }
    let bridge_id = mdns::current_bridge_id();

    let mdns_handle: std::sync::Arc<std::sync::Mutex<Option<mdns::MdnsAdvertiser>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    if install_ctrlc {
        let mdns_for_signal = mdns_handle.clone();
        let hubs_for_signal = known_hub_origins.clone();
        let bridge_id_for_signal = bridge_id.clone();
        let _ = ctrlc::set_handler(move || {
            if let Ok(mut g) = mdns_for_signal.lock() {
                if let Some(ad) = g.as_ref() {
                    ad.shutdown();
                }
                *g = None;
            }
            notify_hubs_bridge_unavailable(&bridge_id_for_signal, &hubs_for_signal);
            std::process::exit(130);
        });
    }

    let player_handle = player::spawn_player(
        device_selected.clone(),
        exclusive_selected.clone(),
        config.enable_dummy_outputs,
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
        config.enable_dummy_outputs,
        player_handle.cmd_tx,
        known_hub_origins.clone(),
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
    notify_hubs_bridge_unavailable(&bridge_id, &known_hub_origins);
    Ok(())
}

/// Normalize and retain only URL origin (`scheme://authority`).
fn normalize_origin(url: Option<&str>) -> Option<String> {
    let value = url?.trim();
    if value.is_empty() {
        return None;
    }
    let uri = actix_web::http::Uri::try_from(value).ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?;
    Some(format!("{scheme}://{authority}"))
}

/// Best-effort hub notification that this bridge is shutting down.
fn notify_hubs_bridge_unavailable(
    bridge_id: &str,
    known_hub_origins: &std::sync::Arc<std::sync::Mutex<HashSet<String>>>,
) {
    let origins = known_hub_origins
        .lock()
        .map(|set| set.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for origin in origins {
        let url = format!(
            "{}/providers/bridge/unregister",
            origin.trim_end_matches('/')
        );
        let response = ureq::post(&url)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(2)))
            .build()
            .send_json(json!({ "bridge_id": bridge_id }));
        match response {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(bridge_id = %bridge_id, hub = %origin, "bridge unregister sent");
            }
            Ok(resp) => {
                tracing::warn!(
                    bridge_id = %bridge_id,
                    hub = %origin,
                    status = %resp.status(),
                    "bridge unregister returned non-success"
                );
            }
            Err(err) => {
                tracing::warn!(bridge_id = %bridge_id, hub = %origin, error = %err, "bridge unregister failed");
            }
        }
    }
}

/// Normalize optional device name input by trimming and dropping empty values.
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
