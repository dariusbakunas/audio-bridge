//! `hub-cli` — a small TUI to stream audio files to `bridge`.
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

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::Sender;
use tracing_subscriber::EnvFilter;

use bridge::config::{BridgeListenConfig, PlaybackConfig};
use bridge::runtime;
use tracing_subscriber::fmt::MakeWriter;

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_SHA"),
    ", ",
    env!("BUILD_DATE"),
    ")"
);

#[derive(Parser, Debug)]
#[command(name = "hub-cli", version = VERSION)]
struct Args {
    /// Base URL of the audio server, e.g. http://192.168.1.10:8080
    #[arg(long)]
    server: String,

    /// Directory on the server to start browsing from.
    #[arg(long, default_value = ".")]
    dir: PathBuf,

    /// Disable the embedded bridge listener.
    #[arg(long, default_value_t = false)]
    no_bridge: bool,

    /// Embedded bridge HTTP API bind address, e.g. 0.0.0.0:5556
    #[arg(long, default_value = "0.0.0.0:5556")]
    bridge_http_bind: SocketAddr,

    /// Use a specific output device by substring match.
    #[arg(long)]
    bridge_device: Option<String>,

    /// Allow insecure TLS connections (self-signed certs).
    #[arg(long, default_value_t = false)]
    tls_insecure: bool,

}

#[derive(Clone)]
struct LogWriter {
    tx: Sender<String>,
    buf: Vec<u8>,
}

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.flush_lines();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_lines();
        Ok(())
    }
}

impl LogWriter {
    fn flush_lines(&mut self) {
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line = self.buf.drain(..=pos).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(&['\r', '\n'][..]);
            if !line.is_empty() {
                let _ = self.tx.send(line.to_string());
            }
        }
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&self.buf);
            let line = line.trim_end_matches(&['\r', '\n'][..]);
            if !line.is_empty() {
                let _ = self.tx.send(line.to_string());
            }
            self.buf.clear();
        }
    }
}

#[derive(Clone)]
struct LogWriterFactory {
    tx: Sender<String>,
}

impl<'a> MakeWriter<'a> for LogWriterFactory {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter {
            tx: self.tx.clone(),
            buf: Vec::new(),
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (log_tx, log_rx) = crossbeam_channel::unbounded::<String>();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,bridge=info,hub_cli=info")
        }))
        .with_writer(LogWriterFactory { tx: log_tx.clone() })
        .with_ansi(false)
        .init();

    tracing::info!(
        version = VERSION,
        server = %args.server,
        bridge_http_bind = %args.bridge_http_bind,
        "hub-cli starting"
    );
    server_api::init_agent(args.tls_insecure);
    if !args.no_bridge {
        let cfg = BridgeListenConfig {
            http_bind: args.bridge_http_bind,
            device: args.bridge_device.clone(),
            playback: PlaybackConfig::default(),
            tls_insecure: args.tls_insecure,
            exclusive_mode: false,
        };
        std::thread::spawn(move || {
            if let Err(e) = runtime::run_listen(cfg, false) {
                tracing::error!("bridge error: {e:#}");
            }
        });
    }

    ui::run_tui(args.server, args.dir, log_rx)
}
