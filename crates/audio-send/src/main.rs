//! Send a single audio file to an `audio-bridge` listener (one file per connection).

use std::fs::File;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "audio-send")]
struct Args {
    /// Address of the receiver, e.g. 192.168.1.10:5555
    #[arg(long)]
    addr: SocketAddr,

    /// Path to the audio file to send
    #[arg(long)]
    path: PathBuf,

    /// Override extension hint (e.g. flac, wav). Defaults to path extension.
    #[arg(long)]
    ext: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut f = File::open(&args.path).with_context(|| format!("open {:?}", args.path))?;

    let ext = args
        .ext
        .as_deref()
        .or_else(|| args.path.extension().and_then(|s| s.to_str()))
        .unwrap_or("");

    let mut stream = TcpStream::connect(args.addr).with_context(|| format!("connect {}", args.addr))?;
    stream.set_nodelay(true).ok();

    audio_bridge_proto::write_header(&mut stream, ext).context("write header")?;

    io::copy(&mut f, &mut stream).context("send file bytes")?;
    // Drop closes the connection; receiver treats EOF as end-of-file.
    Ok(())
}