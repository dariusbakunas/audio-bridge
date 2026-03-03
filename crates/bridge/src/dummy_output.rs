//! Synthetic output devices for bridge testing without physical hardware.
//!
//! Dummy outputs are exposed in `/devices` and can be selected like normal devices.
//! Playback uses a simulated sink that drains decoded audio at real-time speed.

/// Metadata for a synthetic output device.
#[derive(Clone, Debug)]
pub(crate) struct DummyOutputDevice {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) normal_rate_hz: u32,
    pub(crate) exclusive_rate_hz: u32,
    pub(crate) min_rate_hz: u32,
    pub(crate) max_rate_hz: u32,
}

impl DummyOutputDevice {
    /// Effective stream rate for current exclusive-mode selection.
    pub(crate) fn stream_rate_hz(&self, exclusive_mode: bool) -> u32 {
        if exclusive_mode {
            self.exclusive_rate_hz
        } else {
            self.normal_rate_hz
        }
    }
}

const DUMMY_DEVICES: &[DummyOutputDevice] = &[
    DummyOutputDevice {
        id: "dummy:fixed-48k",
        name: "Dummy Output Fixed 48k",
        normal_rate_hz: 48_000,
        exclusive_rate_hz: 48_000,
        min_rate_hz: 48_000,
        max_rate_hz: 48_000,
    },
    DummyOutputDevice {
        id: "dummy:switchable-44k1-96k",
        name: "Dummy Output 44.1k/96k (exclusive)",
        normal_rate_hz: 44_100,
        exclusive_rate_hz: 96_000,
        min_rate_hz: 44_100,
        max_rate_hz: 96_000,
    },
];

/// Return all synthetic devices.
pub(crate) fn list_devices() -> &'static [DummyOutputDevice] {
    DUMMY_DEVICES
}

/// Resolve synthetic device by name.
pub(crate) fn by_name(name: &str) -> Option<DummyOutputDevice> {
    DUMMY_DEVICES.iter().find(|d| d.name == name).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_name_finds_dummy_device() {
        let found = by_name("Dummy Output Fixed 48k");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "dummy:fixed-48k");
    }

    #[test]
    fn stream_rate_switches_with_exclusive_mode() {
        let dev = by_name("Dummy Output 44.1k/96k (exclusive)").unwrap();
        assert_eq!(dev.stream_rate_hz(false), 44_100);
        assert_eq!(dev.stream_rate_hz(true), 96_000);
    }
}
