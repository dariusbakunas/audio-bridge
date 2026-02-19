//! Exclusive (hog) mode helpers for macOS.

#[cfg(target_os = "macos")]
mod macos {
    use cpal::traits::DeviceTrait;
    use coreaudio::audio_unit::macos_helpers::{
        get_device_id_from_name,
        get_hogging_pid,
        set_device_sample_rate,
        toggle_hog_mode,
    };
    use objc2_core_audio::AudioDeviceID;
    use objc2_core_audio::kAudioDevicePropertyNominalSampleRate;
    use objc2_core_audio::kAudioObjectPropertyElementMaster;
    use objc2_core_audio::kAudioObjectPropertyScopeGlobal;
    use objc2_core_audio::AudioObjectGetPropertyData;
    use objc2_core_audio::AudioObjectPropertyAddress;
    use objc2_core_audio::AudioObjectPropertySelector;
    use std::ptr::NonNull;

    pub struct ExclusiveGuard {
        device_id: AudioDeviceID,
        owned: bool,
    }

    impl Drop for ExclusiveGuard {
        fn drop(&mut self) {
            if !self.owned {
                return;
            }
            let pid = get_hogging_pid(self.device_id).unwrap_or(0);
            if pid == std::process::id() as i32 {
                let _ = toggle_hog_mode(self.device_id);
            }
        }
    }

    pub fn maybe_acquire(device: &cpal::Device, sample_rate: u32, enabled: bool) -> Option<ExclusiveGuard> {
        if !enabled {
            return None;
        }
        let name = device.name().ok()?;
        let device_id = match get_device_id_from_name(&name, false) {
            Some(id) => id,
            None => {
                tracing::warn!(device = %name, "exclusive mode: unable to resolve device id");
                return None;
            }
        };

        let pid = get_hogging_pid(device_id).unwrap_or(-1);
        if pid != -1 && pid != std::process::id() as i32 {
            tracing::warn!(
                device = %name,
                hog_pid = pid,
                "exclusive mode: device already hogged by another process"
            );
            return None;
        }

        if pid == -1 {
            match toggle_hog_mode(device_id) {
                Ok(new_pid) if new_pid == std::process::id() as i32 => {}
                Ok(new_pid) => {
                    tracing::warn!(
                        device = %name,
                        hog_pid = new_pid,
                        "exclusive mode: failed to acquire hog mode"
                    );
                    return None;
                }
                Err(err) => {
                    tracing::warn!(
                        device = %name,
                        error = ?err,
                        "exclusive mode: failed to enable hog mode"
                    );
                    return None;
                }
            }
        }

        if let Err(err) = set_device_sample_rate(device_id, sample_rate as f64) {
            tracing::warn!(
                device = %name,
                rate_hz = sample_rate,
                error = ?err,
                "exclusive mode: failed to set device sample rate"
            );
        }

        Some(ExclusiveGuard {
            device_id,
            owned: true,
        })
    }

    pub fn current_nominal_rate(device: &cpal::Device) -> Option<u32> {
        let name = device.name().ok()?;
        let device_id = get_device_id_from_name(&name, false)?;
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate as AudioObjectPropertySelector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let mut rate: f64 = 0.0;
        let data_size = std::mem::size_of::<f64>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                NonNull::from(&property_address),
                0,
                std::ptr::null(),
                NonNull::from(&data_size),
                NonNull::from(&mut rate).cast(),
            )
        };
        if status != 0 {
            return None;
        }
        if rate <= 0.0 {
            return None;
        }
        Some(rate.round() as u32)
    }
}

#[cfg(target_os = "macos")]
pub use macos::{maybe_acquire, current_nominal_rate, ExclusiveGuard};

#[cfg(not(target_os = "macos"))]
pub struct ExclusiveGuard;

#[cfg(not(target_os = "macos"))]
pub fn maybe_acquire(
    _device: &cpal::Device,
    _sample_rate: u32,
    _enabled: bool,
) -> Option<ExclusiveGuard> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn current_nominal_rate(_device: &cpal::Device) -> Option<u32> {
    None
}
