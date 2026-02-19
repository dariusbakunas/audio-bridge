use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use bridge::cli;
use bridge::config::{BridgeListenConfig, BridgePlayConfig, PlaybackConfig};
use bridge::runtime;

const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_SHA"),
    ", ",
    env!("BUILD_DATE"),
    ")"
);

fn main() -> Result<()> {
    let args = cli::Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,bridge=info")
        }))
        .init();

    if args.list_devices {
        runtime::list_devices()?;
        return Ok(());
    }

    tracing::info!(
        version = VERSION,
        http_bind = %args.http_bind,
        device = ?args.device,
        exclusive_mode = args.exclusive_mode,
        "bridge starting"
    );

    let playback = PlaybackConfig {
        chunk_frames: args.chunk_frames,
        refill_max_frames: args.refill_max_frames,
        buffer_seconds: args.buffer_seconds,
    };

    match &args.cmd {
        cli::Command::Play { path } => {
            let cfg = BridgePlayConfig {
                path: path.clone(),
                device: args.device.clone(),
                playback,
                tls_insecure: args.tls_insecure,
                exclusive_mode: args.exclusive_mode,
            };
            runtime::run_play(cfg)?;
        }
        cli::Command::Listen => {
            let cfg = BridgeListenConfig {
                http_bind: args.http_bind,
                device: args.device.clone(),
                playback,
                tls_insecure: args.tls_insecure,
                exclusive_mode: args.exclusive_mode,
            };
            runtime::run_listen(cfg, true)?;
        }
    }

    Ok(())
}
