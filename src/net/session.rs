//! Session handling for the streaming protocol.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{unbounded, Receiver};
use symphonia::core::probe::Hint;

use super::spool::{is_done, make_temp_path, mark_done, Progress};
use super::TEMP_PREFIX;

/// A network session representing exactly one BEGIN_FILE..(FILE_CHUNK..)..(END_FILE or NEXT).
///
/// The TCP connection outlives sessions; sessions are created by the reader thread whenever it
/// sees a new BEGIN_FILE.
#[derive(Debug)]
pub(crate) struct NetSession {
    pub(crate) hint: Hint,
    pub(crate) temp_path: PathBuf,
    pub(crate) control: SessionControl,
    pub(crate) peer_tx: TcpStream,
}

/// Shared control flags for a streaming session (pause/cancel) plus spool progress.
#[derive(Clone, Debug)]
pub(crate) struct SessionControl {
    pub(crate) progress: Arc<(Mutex<Progress>, Condvar)>,
    pub(crate) paused: Arc<AtomicBool>,
    pub(crate) cancel: Arc<AtomicBool>,
}

impl SessionControl {
    pub(crate) fn cancel_and_mark_done(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        mark_done(&self.progress);
    }
}

/// Start handling a single client connection, returning a channel of per-track sessions.
///
/// The returned channel yields one [`NetSession`] per `BEGIN_FILE`.
/// When the client disconnects, the channel closes.
pub(crate) fn run_one_client(mut stream: TcpStream, temp_dir: PathBuf) -> Result<Receiver<NetSession>> {
    // Handshake once per connection.
    audio_bridge_proto::write_prelude(&mut stream).context("write prelude")?;
    audio_bridge_proto::read_prelude(&mut stream).context("read prelude")?;

    // Clone for receiver->sender messages (TrackInfo, PlaybackPos).
    let peer_tx = stream.try_clone().context("try_clone TcpStream for peer_tx")?;

    let (session_tx, session_rx) = unbounded::<NetSession>();

    thread::spawn(move || {
        if let Err(e) = reader_thread_main(stream, peer_tx, session_tx, temp_dir) {
            eprintln!("Connection reader ended: {e:#}");
        }
    });

    Ok(session_rx)
}

fn reader_thread_main(
    mut stream: TcpStream,
    peer_tx: TcpStream,
    session_tx: crossbeam_channel::Sender<NetSession>,
    temp_dir: PathBuf,
) -> Result<()> {
    loop {
        // Wait for the next BEGIN_FILE.
        let (kind, len) = match audio_bridge_proto::read_frame_header(&mut stream) {
            Ok(x) => x,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e).context("read frame header"),
        };

        match kind {
            audio_bridge_proto::FrameKind::BeginFile => {
                let mut payload = vec![0u8; len as usize];
                stream.read_exact(&mut payload).context("read BEGIN_FILE payload")?;
                let ext = audio_bridge_proto::decode_begin_file_payload(&payload).context("decode BEGIN_FILE")?;

                let mut hint = Hint::new();
                if !ext.is_empty() {
                    hint.with_extension(&ext);
                }

                let temp_path = make_temp_path(&temp_dir, TEMP_PREFIX);
                {
                    let _ = OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&temp_path)
                        .with_context(|| format!("create temp file {:?}", temp_path))?;
                }

                let progress: Arc<(Mutex<Progress>, Condvar)> = Arc::new((
                    Mutex::new(Progress {
                        bytes_written: 0,
                        done: false,
                    }),
                    Condvar::new(),
                ));

                let paused = Arc::new(AtomicBool::new(false));
                let cancel = Arc::new(AtomicBool::new(false));
                let control = SessionControl {
                    progress: progress.clone(),
                    paused: paused.clone(),
                    cancel: cancel.clone(),
                };
                let control_for_reader = control.clone();

                // Emit session immediately so playback can start while bytes arrive.
                let session = NetSession {
                    hint,
                    temp_path: temp_path.clone(),
                    control,
                    peer_tx: peer_tx.try_clone().context("try_clone peer_tx for session")?,
                };
                if session_tx.send(session).is_err() {
                    return Ok(());
                }

                // Now read frames for this session.
                //
                // IMPORTANT: after END_FILE we MUST keep reading control frames (NEXT/PAUSE/RESUME)
                // while playback drains, otherwise "next track" won't cancel anything.
                let mut writer = OpenOptions::new()
                    .append(true)
                    .open(&temp_path)
                    .with_context(|| format!("open temp file for append {:?}", temp_path))?;

                loop {
                    let (kind, len) = match audio_bridge_proto::read_frame_header(&mut stream) {
                        Ok(x) => x,
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                            mark_done(&progress);
                            return Ok(());
                        }
                        Err(e) => {
                            mark_done(&progress);
                            return Err(e).context("read frame header");
                        }
                    };

                    match kind {
                        audio_bridge_proto::FrameKind::FileChunk => {
                            let mut buf = vec![0u8; len as usize];
                            stream.read_exact(&mut buf).context("read FILE_CHUNK payload")?;

                            // If already done/canceled, drop bytes but stay in sync.
                            if cancel.load(Ordering::Relaxed) || is_done(&progress) {
                                continue;
                            }

                            writer.write_all(&buf).context("write to temp file")?;
                            writer.flush().ok();

                            let (lock, cv) = &*progress;
                            let mut g = lock.lock().unwrap();
                            g.bytes_written = g.bytes_written.saturating_add(buf.len() as u64);
                            drop(g);
                            cv.notify_all();
                        }

                        audio_bridge_proto::FrameKind::EndFile => {
                            drain_payload(&mut stream, len);
                            // Mark spooling done, but DO NOT exit the session loop.
                            // We keep handling control frames while playback drains.
                            mark_done(&progress);
                        }

                        audio_bridge_proto::FrameKind::Pause => {
                            drain_payload(&mut stream, len);
                            paused.store(true, Ordering::Relaxed);
                        }

                        audio_bridge_proto::FrameKind::Resume => {
                            drain_payload(&mut stream, len);
                            paused.store(false, Ordering::Relaxed);
                        }

                        audio_bridge_proto::FrameKind::Next => {
                            drain_payload(&mut stream, len);

                            // Hard cut: cancel playback + stop caring about this session immediately.
                            control_for_reader.cancel_and_mark_done();
                            break; // back to outer loop, waiting for next BEGIN_FILE
                        }

                        audio_bridge_proto::FrameKind::BeginFile => {
                            // Sender started a new track without an explicit NEXT.
                            // Treat this as a hard cut + immediately start the next session.
                            //
                            // We already consumed the header; now consume its payload and then
                            // cancel this session and "replay" by switching to the outer loop logic.
                            drain_payload(&mut stream, len);

                            control_for_reader.cancel_and_mark_done();
                            break;
                        }

                        audio_bridge_proto::FrameKind::Error => {
                            let mut msg = vec![0u8; len as usize];
                            stream.read_exact(&mut msg).ok();
                            control_for_reader.cancel_and_mark_done();
                            return Err(anyhow!("Sender sent ERROR: {}", String::from_utf8_lossy(&msg)));
                        }

                        audio_bridge_proto::FrameKind::TrackInfo | audio_bridge_proto::FrameKind::PlaybackPos => {
                            drain_payload(&mut stream, len);
                        }
                    }
                }
            }

            // Ignore control frames while idle (no current session).
            audio_bridge_proto::FrameKind::Pause
            | audio_bridge_proto::FrameKind::Resume
            | audio_bridge_proto::FrameKind::Next => {
                drain_payload(&mut stream, len);
                continue;
            }

            other => {
                drain_payload(&mut stream, len);
                eprintln!("Ignoring unexpected frame while idle: {other:?}");
                continue;
            }
        }
    }
}

/// Read and discard a payload of length `len` to keep the stream in sync.
fn drain_payload(stream: &mut TcpStream, len: u32) {
    if len == 0 {
        return;
    }

    let mut junk = vec![0u8; len as usize];
    let _ = stream.read_exact(&mut junk);
}

/// Accept a single TCP connection from `listener`.
pub(crate) fn accept_one(listener: &TcpListener) -> Result<TcpStream> {
    let (stream, addr) = listener.accept().context("accept connection")?;
    eprintln!("Client connected: {addr}");
    stream
        .set_nodelay(true)
        .ok(); // best-effort; not fatal
    Ok(stream)
}
