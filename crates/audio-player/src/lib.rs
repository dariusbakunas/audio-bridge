//! Audio pipeline building blocks shared by bridge + server.
//!
//! This crate focuses on decoding, resampling, queueing, and playback wiring.
//! Callers provide I/O and control logic; the audio path stays reusable.
//!
//! # Example
//! ```no_run
//! use audio_player::{config::PlaybackConfig, decode, device, pipeline};
//!
//! let host = cpal::default_host();
//! let device = device::pick_device(&host, None).expect("device");
//! let (spec, srcq, _duration_ms, _source_info) =
//!     decode::start_streaming_decode(&std::path::PathBuf::from("song.flac"), 2.0)
//!         .expect("decode");
//! let config = device::pick_output_config(&device, Some(spec.rate)).expect("config");
//! let stream_config: cpal::StreamConfig = config.clone().into();
//! let playback = PlaybackConfig::default();
//! pipeline::play_decoded_source(
//!     &device,
//!     &config,
//!     &stream_config,
//!     &playback,
//!     spec,
//!     srcq,
//!     pipeline::PlaybackSessionOptions {
//!         paused: None,
//!         cancel: None,
//!         played_frames: None,
//!         underrun_frames: None,
//!         underrun_events: None,
//!         buffered_frames: None,
//!         buffer_capacity_frames: None,
//!         volume_percent: None,
//!         muted: None,
//!     },
//! ).expect("playback");
//! ```

/// Shared playback tuning parameters.
pub mod config;
pub mod decode;
pub mod device;
pub mod pipeline;
pub mod playback;
pub mod queue;
pub mod resample;
/// Playback status snapshot helpers shared with API layers.
pub mod status;
