//! Command-line interface definitions.
//!
//! This module contains the `clap`-powered CLI surface area (args + defaults).
//! It intentionally has no audio logic so the rest of the crate can stay reusable.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to audio file (FLAC recommended)
    pub path: PathBuf,

    /// List output devices and exit
    #[arg(long)]
    pub list_devices: bool,

    /// Use a specific output device by substring match
    #[arg(long)]
    pub device: Option<String>,

    /// Resampler input chunk size in frames (higher => more latency, lower => more overhead)
    #[arg(long, default_value_t = 1024)]
    pub chunk_frames: usize,

    /// Playback callback refill cap (frames). Larger reduces lock churn but can add latency.
    #[arg(long, default_value_t = 4096)]
    pub refill_max_frames: usize,

    /// Queue buffer target in seconds (per stage)
    #[arg(long, default_value_t = 2.0)]
    pub buffer_seconds: f32,
}