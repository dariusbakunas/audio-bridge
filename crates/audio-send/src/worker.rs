//! Background worker that owns the TCP connection and streams file bytes.
//!
//! Design goals:
//! - UI always stays responsive.
//! - "Next" is immediate: abort current send by dropping the socket and reconnecting.
//! - Pause/resume uses protocol frames; receiver pauses playback by not draining audio.

use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

#[derive(Debug, Clone)]
pub enum Command {
    Play { path: PathBuf, ext_hint: String },
    PauseToggle,
    Next,
    Quit,
}

#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    Progress { sent: u64, total: Option<u64> },
    Error(String),
}

pub fn worker_main(addr: SocketAddr, cmd_rx: Receiver<Command>, evt_tx: Sender<Event>) {
    let mut paused = false;

    // "Next" is handled as "abort current send". We'll also treat Play as "abort and play new".
    loop {
        let cmd = match cmd_rx.recv() {
            Ok(c) => c,
            Err(_) => break,
        };

        match cmd {
            Command::Quit => break,

            Command::PauseToggle => {
                paused = !paused;
                // If we're not currently sending, we still keep the paused state for the next track.
                let _ = evt_tx.send(Event::Status(if paused { "Paused".into() } else { "Playing".into() }));
            }

            Command::Next => {
                // No-op at top-level: Next only matters during a send loop.
                let _ = evt_tx.send(Event::Status("Next requested".into()));
            }

            Command::Play { path, ext_hint } => {
                if let Err(e) = send_one_track_session(
                    addr,
                    &cmd_rx,
                    &evt_tx,
                    &mut paused,
                    path,
                    ext_hint,
                ) {
                    let _ = evt_tx.send(Event::Error(format!("{e:#}")));
                }
            }
        }
    }
}

fn send_one_track_session(
    addr: SocketAddr,
    cmd_rx: &Receiver<Command>,
    evt_tx: &Sender<Event>,
    paused: &mut bool,
    path: PathBuf,
    ext_hint: String,
) -> Result<()> {
    evt_tx
        .send(Event::Status(format!("Connecting to {addr}...")))
        .ok();

    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();

    // Important: allows us to remain responsive under receiver pause/backpressure.
    stream
        .set_write_timeout(Some(Duration::from_millis(200)))
        .ok();

    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint).context("encode BEGIN_FILE")?;
    audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::BeginFile, &begin)
        .context("write BEGIN_FILE")?;

    // Apply current paused state immediately.
    if *paused {
        audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Pause, &[])
            .context("write PAUSE")?;
    }

    evt_tx
        .send(Event::Status(format!(
            "Sending: {}",
            path.file_name().and_then(|s| s.to_str()).unwrap_or("<file>")
        )))
        .ok();

    let meta_len = std::fs::metadata(&path).ok().map(|m| m.len());

    let mut f = File::open(&path).with_context(|| format!("open {:?}", path))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut sent: u64 = 0;

    'send_loop: loop {
        // Handle any pending commands without blocking.
        loop {
            match cmd_rx.try_recv() {
                Ok(Command::Quit) => {
                    evt_tx.send(Event::Status("Quit".into())).ok();
                    return Ok(());
                }
                Ok(Command::Next) => {
                    evt_tx.send(Event::Status("Skipping (next)".into())).ok();
                    // Tell receiver to stop listening for controls for this file, then drop socket.
                    let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                    return Ok(());
                }
                Ok(Command::Play { .. }) => {
                    evt_tx.send(Event::Status("Switching track".into())).ok();
                    let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                    return Ok(());
                }
                Ok(Command::PauseToggle) => {
                    *paused = !*paused;
                    let kind = if *paused {
                        audio_bridge_proto::FrameKind::Pause
                    } else {
                        audio_bridge_proto::FrameKind::Resume
                    };
                    audio_bridge_proto::write_frame(&mut stream, kind, &[])
                        .with_context(|| format!("write {kind:?}"))?;
                    evt_tx.send(Event::Status(if *paused { "Paused".into() } else { "Playing".into() })).ok();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        let n = f.read(&mut buf).context("read file")?;
        if n == 0 {
            break 'send_loop;
        }

        match audio_bridge_proto::write_frame(
            &mut stream,
            audio_bridge_proto::FrameKind::FileChunk,
            &buf[..n],
        ) {
            Ok(()) => {
                sent = sent.saturating_add(n as u64);
                evt_tx.send(Event::Progress { sent, total: meta_len }).ok();
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                continue 'send_loop;
            }
            Err(e) => return Err(e).context("write FILE_CHUNK").map_err(Into::into),
        }
    }

    audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::EndFile, &[])
        .context("write END_FILE")?;

    evt_tx.send(Event::Status("Sent (remote playing; controls active)".into())).ok();

    // Control-only phase: keep connection open so PAUSE/RESUME works during playback.
    loop {
        match cmd_rx.recv() {
            Ok(Command::Quit) => {
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(());
            }
            Ok(Command::Next) => {
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(());
            }
            Ok(Command::Play { .. }) => {
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(());
            }
            Ok(Command::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                // If receiver already closed, this can error; treat it as end of session.
                if audio_bridge_proto::write_frame(&mut stream, kind, &[]).is_err() {
                    return Ok(());
                }
                evt_tx.send(Event::Status(if *paused { "Paused".into() } else { "Playing".into() })).ok();
            }
            Err(_) => {
                // UI side dropped the command channel; end the session cleanly.
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(());
            }
        }
    }
}