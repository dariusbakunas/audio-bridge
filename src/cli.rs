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
}