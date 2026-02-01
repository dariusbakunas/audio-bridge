use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::state::PlayerStatus;

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Play { path: PathBuf, ext_hint: String },
    PauseToggle,
    Next,
}

#[derive(Clone)]
pub struct BridgePlayer {
    pub(crate) cmd_tx: Sender<BridgeCommand>,
}

pub fn spawn_bridge_worker(
    addr: SocketAddr,
    cmd_rx: Receiver<BridgeCommand>,
    status: Arc<Mutex<PlayerStatus>>,
) {
    std::thread::spawn(move || {
        let mut stream = connect_loop(addr, status.clone());
        let mut paused = false;

        loop {
            let cmd = match cmd_rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            };

            match cmd {
                BridgeCommand::PauseToggle => {
                    paused = !paused;
                    let kind = if paused {
                        audio_bridge_proto::FrameKind::Pause
                    } else {
                        audio_bridge_proto::FrameKind::Resume
                    };
                    if audio_bridge_proto::write_frame(&mut stream, kind, &[]).is_ok() {
                        if let Ok(mut s) = status.lock() {
                            s.paused = paused;
                        }
                    }
                }
                BridgeCommand::Next => {
                    let _ = audio_bridge_proto::write_frame(
                        &mut stream,
                        audio_bridge_proto::FrameKind::Next,
                        &[],
                    );
                    if let Ok(mut s) = status.lock() {
                        s.paused = false;
                    }
                }
                BridgeCommand::Play { path, ext_hint } => {
                    let mut next = Some((path, ext_hint));
                    while let Some((path, ext_hint)) = next.take() {
                        if let Ok(mut s) = status.lock() {
                            s.now_playing = Some(path.clone());
                            s.paused = false;
                            s.elapsed_ms = Some(0);
                        }
                        match send_one_track_over_existing_connection(
                            &mut stream,
                            &cmd_rx,
                            &mut paused,
                            path,
                            ext_hint,
                        ) {
                            Ok(Flow::Continue) => break,
                            Ok(Flow::SwitchTo { path, ext_hint }) => {
                                next = Some((path, ext_hint));
                            }
                            Err(_) => {
                                stream = connect_loop(addr, status.clone());
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
}

#[derive(Debug)]
enum Flow {
    Continue,
    SwitchTo { path: PathBuf, ext_hint: String },
}

fn connect_loop(addr: SocketAddr, status: Arc<Mutex<PlayerStatus>>) -> TcpStream {
    let mut delay = Duration::from_millis(250);
    loop {
        match connect_and_handshake(addr, status.clone()) {
            Ok(stream) => return stream,
            Err(_) => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

fn connect_and_handshake(addr: SocketAddr, status: Arc<Mutex<PlayerStatus>>) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(addr).with_context(|| format!("connect {addr}"))?;
    stream.set_nodelay(true).ok();
    stream.set_write_timeout(Some(Duration::from_millis(200))).ok();

    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;
    let mut stream_rx = stream.try_clone().context("try_clone stream for rx")?;
    audio_bridge_proto::read_prelude(&mut stream_rx).context("read prelude")?;
    spawn_bridge_reader(stream_rx, status);
    Ok(stream)
}

fn spawn_bridge_reader(mut stream_rx: TcpStream, status: Arc<Mutex<PlayerStatus>>) {
    std::thread::spawn(move || loop {
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
                if let Ok((sr, _ch, dur)) = audio_bridge_proto::decode_track_info(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.sample_rate = Some(sr);
                        if dur.is_some() {
                            s.duration_ms = dur;
                        }
                    }
                }
            }
            audio_bridge_proto::FrameKind::PlaybackPos => {
                if let Ok((frames, paused)) = audio_bridge_proto::decode_playback_pos(&payload) {
                    if let Ok(mut s) = status.lock() {
                        s.paused = paused;
                        if let Some(sr) = s.sample_rate {
                            if sr > 0 {
                                s.elapsed_ms = Some(frames.saturating_mul(1000) / sr as u64);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    });
}

fn write_all_interruptible(
    stream: &mut TcpStream,
    buf: &[u8],
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
) -> Result<Flow> {
    let mut off = 0usize;

    while off < buf.len() {
        match cmd_rx.try_recv() {
            Ok(BridgeCommand::Next) => return Ok(Flow::Continue),
            Ok(BridgeCommand::Play { path, ext_hint }) => return Ok(Flow::SwitchTo { path, ext_hint }),
            Ok(BridgeCommand::PauseToggle) => {
                *paused = !*paused;
                let kind = if *paused {
                    audio_bridge_proto::FrameKind::Pause
                } else {
                    audio_bridge_proto::FrameKind::Resume
                };
                audio_bridge_proto::write_frame(&mut *stream, kind, &[])
                    .with_context(|| format!("write {kind:?}"))?;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Ok(Flow::Continue),
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
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
) -> Result<Flow> {
    let frame = audio_bridge_proto::encode_frame(kind, payload)
        .context("encode frame")?;
    write_all_interruptible(stream, &frame, cmd_rx, paused)
}

fn send_one_track_over_existing_connection(
    stream: &mut TcpStream,
    cmd_rx: &Receiver<BridgeCommand>,
    paused: &mut bool,
    path: PathBuf,
    ext_hint: String,
) -> Result<Flow> {
    let _ = audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Next, &[]);

    let begin = audio_bridge_proto::encode_begin_file_payload(&ext_hint)
        .context("encode BEGIN_FILE")?;
    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::BeginFile,
        &begin,
        cmd_rx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    if *paused {
        audio_bridge_proto::write_frame(&mut *stream, audio_bridge_proto::FrameKind::Pause, &[])
            .context("write PAUSE")?;
    }

    let mut file = File::open(&path).with_context(|| format!("open {:?}", path))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("read file")?;
        if n == 0 {
            break;
        }
        match write_frame_interruptible(
            stream,
            audio_bridge_proto::FrameKind::FileChunk,
            &buf[..n],
            cmd_rx,
            paused,
        )? {
            Flow::Continue => {}
            other => return Ok(other),
        }
    }

    match write_frame_interruptible(
        stream,
        audio_bridge_proto::FrameKind::EndFile,
        &[],
        cmd_rx,
        paused,
    )? {
        Flow::Continue => {}
        other => return Ok(other),
    }

    Ok(Flow::Continue)
}
