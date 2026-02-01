//! `audio-send` — a small TUI to stream audio files to `audio-bridge`.
//!
//! Features:
//! - list `.flac`/`.wav` files in current directory (non-recursive)
//! - Enter: play selected (immediately starts sending)
//! - Space: pause/resume (sends PAUSE/RESUME frames)
//! - n: next (skip immediately)
//! - q: quit

mod library;
mod server_api;
mod ui;
mod worker;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "audio-send")]
struct Args {
    /// Base URL of the audio server, e.g. http://192.168.1.10:8080
    #[arg(long)]
    server: String,

    /// Directory on the server to start browsing from.
    #[arg(long, default_value = ".")]
    dir: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ui::run_tui(args.server, args.dir)
}
