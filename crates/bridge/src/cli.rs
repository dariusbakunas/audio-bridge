//! Command-line interface definitions.
//!
//! This module contains the `clap`-powered CLI surface area (args + defaults).
//! It intentionally has no audio logic so the rest of the crate can stay reusable.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bridge")]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Command,

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

    /// Temp directory for streamed files (defaults to OS temp dir)
    #[arg(long)]
    pub temp_dir: Option<PathBuf>,

    /// HTTP API bind address, e.g. 0.0.0.0:5556
    #[arg(long, default_value = "0.0.0.0:5556")]
    pub http_bind: SocketAddr,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Play a local file (current behavior)
    Play {
        /// Path to audio file (FLAC recommended)
        path: PathBuf,
    },

    /// Listen for a TCP connection and play exactly one streamed file per connection
    Listen {
        /// Address to bind, e.g. 0.0.0.0:5555
        #[arg(long, default_value = "0.0.0.0:5555")]
        bind: SocketAddr,
    },
}
