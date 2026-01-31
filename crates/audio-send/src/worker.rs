//! Background worker that owns the TCP connection and streams file bytes.
//!
//! Design goals:
//! - UI always stays responsive.
//! - "Next" is immediate: cancel current session via NEXT frame (no reconnect).
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

#[derive(Debug)]
enum Flow {
    Continue,
    SwitchTo { path: PathBuf, ext_hint: String },
    Quit,
}

fn write_all_interruptible(
    stream: &mut TcpStream,
    buf: &[u8],
    cmd_rx: &Receiver<Command>,
    evt_tx: &Sender<Event>,
    paused: &mut bool,
) -> Result<Flow> {
    let mut off = 0usize;

    while off < buf.len() {
        match cmd_rx.try_recv() {
            Ok(Command::Quit) => return Ok(Flow::Quit),
            Ok(Command::Next) => return Ok(Flow::Continue), // caller will send NEXT
            Ok(Command::Play { path, ext_hint }) => return Ok(Flow::SwitchTo { path, ext_hint }),
            Ok(Command::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                audio_bridge_proto::write_frame(&mut *stream, kind, &[])
                    .with_context(|| format!("write {kind:?}"))?;
                evt_tx
                    .send(Event::Status(if *paused { "Paused".into() } else { "Playing".into() }))
                    .ok();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Ok(Flow::Quit),
        }

        match stream.write(&buf[off..]) {
            Ok(0) => return Err(anyhow::anyhow!("socket closed while writing")).map_err(Into::into),
            Ok(n) => off += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e).context("write to socket").map_err(Into::into),
        }
    }

    Ok(Flow::Continue)
}

fn write_frame_interruptible(
    stream: &mut TcpStream,
    kind: audio_bridge_proto::FrameKind,
    payload: &[u8],
    cmd_rx: &Receiver<Command>,
    evt_tx: &Sender<Event>,
    paused: &mut bool,
) -> Result<Flow> {
    let frame = audio_bridge_proto::encode_frame(kind, payload)
        .context("encode frame")?;
    write_all_interruptible(stream, &frame, cmd_rx, evt_tx, paused)
}

fn connect_and_spawn_rx(addr: SocketAddr, evt_tx: &Sender<Event>) -> Result<TcpStream> {
    evt_tx.send(Event::Status(format!("Connecting to {addr}..."))).ok();

    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_millis(200))).ok();

    // Handshake once per connection.
    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;
    let mut stream_rx = stream.try_clone().context("try_clone stream for rx")?;
    audio_bridge_proto::read_prelude(&mut stream_rx).context("read prelude")?;

    // One long-lived read-back thread: receiver -> sender frames.
    let evt_tx_rx = evt_tx.clone();
    std::thread::spawn(move || {
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
                        let _ = evt_tx_rx.send(Event::RemotePlaybackPos {
                            played_frames: frames,
                            paused,
                        });
                    }
                }
                _ => {}
            }
        }
    });

    evt_tx.send(Event::Status("Connected".into())).ok();
    Ok(stream)
}

fn send_one_track_over_existing_connection(
    stream: &mut TcpStream,
    cmd_rx: &Receiver<Command>,
    evt_tx: &Sender<Event>,
    paused: &mut bool,
    path: PathBuf,
    ext_hint: String,
) -> Result<Flow> {
    // Hard cut whatever the receiver is doing right now.
    let _ = audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Next, &[]);

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint).context("encode BEGIN_FILE")?;
    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::BeginFile,
        &begin,
        cmd_rx,
        evt_tx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    if *paused {
        audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Pause, &[])
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

    loop {
        let n = f.read(&mut buf).context("read file")?;
        if n == 0 {
            break;
        }

        match write_frame_interruptible(
            stream,
            audio_bridge_proto::FrameKind::FileChunk,
            &buf[..n],
            cmd_rx,
            evt_tx,
            paused,
        )? {
            Flow::Continue => {
                sent = sent.saturating_add(n as u64);
                evt_tx.send(Event::Progress { sent, total: meta_len }).ok();
            }
            Flow::Quit => return Ok(Flow::Quit),
            Flow::SwitchTo { path, ext_hint } => {
                // Cancel current session at receiver, then switch immediately.
                let _ = audio_bridge_proto::write_frame(stream, audio_bridge_proto::FrameKind::Next, &[]);
                return Ok(Flow::SwitchTo { path, ext_hint });
            }
        }
    }

    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::EndFile,
        &[],
        cmd_rx,
        evt_tx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    evt_tx.send(Event::Status("Sent (remote playing; controls active)".into())).ok();
    Ok(Flow::Continue)
}

pub fn worker_main(addr: SocketAddr, cmd_rx: Receiver<Command>, evt_tx: Sender<Event>) {
    let mut paused = false;

    let mut stream = match connect_and_spawn_rx(addr, &evt_tx) {
        Ok(s) => s,
        Err(e) => {
            let _ = evt_tx.send(Event::Error(format!("{e:#}")));
            return;
        }
    };

    // Main control loop: no reconnects; just reuse the same stream.
    loop {
        let cmd = match cmd_rx.recv() {
            Ok(c) => c,
            Err(_) => break,
        };

        match cmd {
            Command::Quit => {
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                break;
            }
            Command::PauseToggle => {
                paused = !paused;
                let kind = if paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                let _ = audio_bridge_proto::write_frame(&mut stream, kind, &[]);
                let _ = evt_tx.send(Event::Status(if paused { "Paused".into() } else { "Playing".into() }));
            }
            Command::Next => {
                paused = false;
                let _ = audio_bridge_proto::write_frame(&mut stream, audio_bridge_proto::FrameKind::Next, &[]);
                let _ = evt_tx.send(Event::Status("Skipping (next)".into()));
            }
            Command::Play { path, ext_hint } => {
                // New track should start playing (do not carry pause across tracks).
                paused = false;

                // Drive a sequence of immediate switches without requiring extra keypresses.
                let mut pending = Some((path, ext_hint));
                while let Some((p, e)) = pending.take() {
                    match send_one_track_over_existing_connection(
                        &mut stream,
                        &cmd_rx,
                        &evt_tx,
                        &mut paused,
                        p,
                        e,
                    ) {
                        Ok(Flow::Continue) => {}
                        Ok(Flow::Quit) => return,
                        Ok(Flow::SwitchTo { path, ext_hint }) => {
                            // Switching tracks should also start unpaused.
                            paused = false;
                            pending = Some((path, ext_hint));
                        }
                        Err(err) => {
                            let _ = evt_tx.send(Event::Error(format!("{err:#}")));
                            return;
                        }
                    }
                }
            }
        }
    }
}
