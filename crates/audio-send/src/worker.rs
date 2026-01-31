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
    RemoteTrackInfo { sample_rate: u32, channels: u16, duration_ms: Option<u64> },
    RemotePlaybackPos { played_frames: u64, paused: bool },
    Error(String),
}

/// What `send_one_track_session` wants the outer loop to do next.
#[derive(Debug)]
enum SessionOutcome {
    /// End the current session and wait for the next command.
    Done,
    /// Immediately start another track without requiring another keypress.
    SwitchTo { path: PathBuf, ext_hint: String },
    /// Quit the worker.
    Quit,
}

pub fn worker_main(addr: SocketAddr, cmd_rx: Receiver<Command>, evt_tx: Sender<Event>) {
    let mut paused = false;
    let mut pending: Option<Command> = None;

    loop {
        let cmd = if let Some(c) = pending.take() {
            c
        } else {
            match cmd_rx.recv() {
                Ok(c) => c,
                Err(_) => break,
            }
        };

        match cmd {
            Command::Quit => break,

            Command::PauseToggle => {
                paused = !paused;
                let _ = evt_tx.send(Event::Status(if paused { "Paused".into() } else { "Playing".into() }));
            }

            Command::Next => {
                let _ = evt_tx.send(Event::Status("Next requested".into()));
            }

            Command::Play { path, ext_hint } => {
                match send_one_track_session(addr, &cmd_rx, &evt_tx, &mut paused, path, ext_hint) {
                    Ok(SessionOutcome::Done) => {}
                    Ok(SessionOutcome::Quit) => break,
                    Ok(SessionOutcome::SwitchTo { path, ext_hint }) => {
                        pending = Some(Command::Play { path, ext_hint });
                    }
                    Err(e) => {
                        let _ = evt_tx.send(Event::Error(format!("{e:#}")));
                    }
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
) -> Result<SessionOutcome> {
    evt_tx.send(Event::Status(format!("Connecting to {addr}..."))).ok();

    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_millis(200))).ok();

    // Read-back thread for receiver -> sender frames (optional; keep if you already added it).
    let mut stream_rx = stream.try_clone().context("try_clone stream for rx")?;
    let evt_tx_rx = evt_tx.clone();
    std::thread::spawn(move || {
        if audio_bridge_proto::read_prelude(&mut stream_rx).is_err() {
            return;
        }
        loop {
            let (kind, len) = match audio_bridge_proto::read_frame_header(&mut stream_rx) {
                Ok(x) => x,
                Err(_) => break,
            };
            let mut payload = vec![0u8; len as usize];
            if stream_rx.read_exact(&mut payload).is_err() {
                break;
            }
            match kind {
                audio_bridge_proto::FrameKind::TrackInfo => {
                    if let Ok((sr, ch, dur)) = audio_bridge_proto::decode_track_info(&payload) {
                        let _ = evt_tx_rx.send(Event::RemoteTrackInfo {
                            sample_rate: sr,
                            channels: ch,
                            duration_ms: dur,
                        });
                    }
                }
                audio_bridge_proto::FrameKind::PlaybackPos => {
                    if let Ok((frames, paused)) = audio_bridge_proto::decode_playback_pos(&payload) {
                        let _ = evt_tx_rx.send(Event::RemotePlaybackPos { played_frames: frames, paused });
                    }
                }
                _ => {}
            }
        }
    });

    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint).context("encode BEGIN_FILE")?;
    audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::BeginFile, &begin)
        .context("write BEGIN_FILE")?;

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
        loop {
            match cmd_rx.try_recv() {
                Ok(Command::Quit) => {
                    evt_tx.send(Event::Status("Quit".into())).ok();
                    return Ok(SessionOutcome::Quit);
                }
                Ok(Command::Next) => {
                    evt_tx.send(Event::Status("Skipping (next)".into())).ok();
                    *paused = false; // don't carry pause into the next track
                    let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                    return Ok(SessionOutcome::Done);
                }
                Ok(Command::Play { path, ext_hint }) => {
                    evt_tx.send(Event::Status("Switching track".into())).ok();
                    *paused = false; // new track should start playing immediately
                    let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                    return Ok(SessionOutcome::SwitchTo { path, ext_hint });
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
                Err(TryRecvError::Disconnected) => return Ok(SessionOutcome::Quit),
            }
        }

        let n = f.read(&mut buf).context("read file")?;
        if n == 0 {
            break 'send_loop;
        }

        match audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::FileChunk, &buf[..n]) {
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
                return Ok(SessionOutcome::Quit);
            }
            Ok(Command::Next) => {
                *paused = false; // next track should start unpaused
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(SessionOutcome::Done);
            }
            Ok(Command::Play { path, ext_hint }) => {
                *paused = false; // new track should start unpaused
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(SessionOutcome::SwitchTo { path, ext_hint });
            }
            Ok(Command::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                if audio_bridge_proto::write_frame(&mut stream, kind, &[]).is_err() {
                    return Ok(SessionOutcome::Done);
                }
                evt_tx.send(Event::Status(if *paused { "Paused".into() } else { "Playing".into() })).ok();
            }
            Err(_) => {
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(SessionOutcome::Quit);
            }
        }
    }
}