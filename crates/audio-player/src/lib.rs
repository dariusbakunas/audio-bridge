//! Audio pipeline building blocks shared by bridge + server.
//!
//! This crate focuses on decoding, resampling, queueing, and playback wiring.
//! Callers provide I/O and control logic; the audio path stays reusable.

pub mod config;
pub mod decode;
pub mod device;
pub mod pipeline;
pub mod playback;
pub mod queue;
pub mod resample;
pub mod status;
