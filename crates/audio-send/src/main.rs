//! `audio-send` — a small TUI to stream audio files to `audio-bridge`.
//!
//! Features:
//! - list `.flac`/`.wav` files in current directory (non-recursive)
//! - Enter: play selected (immediately starts sending)
//! - Space: pause/resume (sends PAUSE/RESUME frames)
//! - n: next (skip immediately)
//! - q: quit

mod library;
mod ui;
mod worker;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "audio-send")]
struct Args {
    /// Address of the receiver, e.g. 192.168.1.10:5555
    #[arg(long)]
    addr: SocketAddr,

    /// Directory to scan for audio files (non-recursive). Defaults to current directory.
    #[arg(long, default_value = ".")]
    dir: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ui::run_tui(args.addr, args.dir)
}
