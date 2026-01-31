//! Network receiver utilities for “one file per connection” streaming.
//!
//! The receiver spools incoming bytes to a temp file while simultaneously providing a
//! blocking, seekable reader to Symphonia. This allows “start playback as soon as enough
//! bytes arrive” without requiring the socket itself to be seekable.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use symphonia::core::io::MediaSource;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone)]
pub(crate) struct IncomingStreamInfo {
    pub(crate) hint: Hint,
    pub(crate) temp_path: PathBuf,
}

#[derive(Debug)]
struct Progress {
    bytes_written: u64,
    done: bool,
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

/// Receive exactly one streamed file over a framed connection (protocol v2),
/// spooling it to a temp file, and return a MediaSource that blocks until bytes are available.
///
/// Control frames (PAUSE/RESUME) update the returned `paused` flag.
///
/// The file ends when `END_FILE` is received (not EOF).
pub(crate) fn recv_one_framed_file_as_media_source(
    mut stream: TcpStream,
) -> Result<(IncomingStreamInfo, Box<dyn MediaSource>, Arc<AtomicBool>)> {
    audio_bridge_proto::read_prelude(&mut stream).context("read prelude")?;

    let paused = Arc::new(AtomicBool::new(false));

    // Expect BEGIN_FILE first.
    let (kind, len) = audio_bridge_proto::read_frame_header(&mut stream).context("read frame header")?;
    if kind != audio_bridge_proto::FrameKind::BeginFile {
        return Err(anyhow!("Expected BEGIN_FILE, got {kind:?}"));
    }

    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload).context("read BEGIN_FILE payload")?;
    let ext = audio_bridge_proto::decode_begin_file_payload(&payload).context("decode BEGIN_FILE")?;

    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(&ext);
    }

    let temp_path = make_temp_path("audio-bridge-stream");
    let writer_path = temp_path.clone();

    {
        let _ = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&writer_path)
            .with_context(|| format!("create temp file {:?}", writer_path))?;
    }

    let progress: Arc<(Mutex<Progress>, Condvar)> = Arc::new((
        Mutex::new(Progress {
            bytes_written: 0,
            done: false,
        }),
        Condvar::new(),
    ));

    let progress_writer = progress.clone();
    let paused_writer = paused.clone();
    thread::spawn(move || {
        if let Err(e) = writer_thread_main_framed(&mut stream, &writer_path, &progress_writer, &paused_writer) {
            eprintln!("Network writer error: {e:#}");
        }
        let (lock, cv) = &*progress_writer;
        let mut g = lock.lock().unwrap();
        g.done = true;
        drop(g);
        cv.notify_all();
    });

    let file_for_read = OpenOptions::new()
        .read(true)
        .open(&temp_path)
        .with_context(|| format!("open temp file for read {:?}", temp_path))?;

    let source = Box::new(BlockingFileSource::new(file_for_read, progress.clone()));

    Ok((
        IncomingStreamInfo { hint, temp_path },
        source,
        paused,
    ))
}

fn writer_thread_main_framed(
    stream: &mut TcpStream,
    path: &PathBuf,
    progress: &Arc<(Mutex<Progress>, Condvar)>,
    paused: &Arc<AtomicBool>,
) -> Result<()> {
    let mut f = OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("open temp file for append {:?}", path))?;

    loop {
        let (kind, len) = match audio_bridge_proto::read_frame_header(&mut *stream) {
            Ok(x) => x,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e).context("read frame header"),
        };

        match kind {
            audio_bridge_proto::FrameKind::FileChunk => {
                // If we've already finished the file (END_FILE), we should not receive more chunks.
                // Drain payload to stay in sync and ignore.
                let (lock, _) = &**progress;
                let done_already = lock.lock().unwrap().done;
                drop(lock);

                let mut buf = vec![0u8; len as usize];
                stream
                    .read_exact(&mut buf)
                    .context("read FILE_CHUNK payload")?;

                if done_already {
                    continue;
                }

                f.write_all(&buf).context("write to temp file")?;
                f.flush().ok();

                let (lock, cv) = &**progress;
                let mut g = lock.lock().unwrap();
                g.bytes_written = g.bytes_written.saturating_add(buf.len() as u64);
                drop(g);
                cv.notify_all();
            }
            audio_bridge_proto::FrameKind::EndFile => {
                // Mark the file as complete so the MediaSource reaches EOF,
                // but keep the TCP connection open to accept PAUSE/RESUME/NEXT.
                if len != 0 {
                    let mut junk = vec![0u8; len as usize];
                    stream.read_exact(&mut junk).ok();
                }

                let (lock, cv) = &**progress;
                let mut g = lock.lock().unwrap();
                g.done = true;
                drop(g);
                cv.notify_all();

                // Continue loop (control frames may follow).
            }
            audio_bridge_proto::FrameKind::Pause => {
                if len != 0 {
                    let mut junk = vec![0u8; len as usize];
                    stream.read_exact(&mut junk).ok();
                }
                paused.store(true, Ordering::Relaxed);
            }
            audio_bridge_proto::FrameKind::Resume => {
                if len != 0 {
                    let mut junk = vec![0u8; len as usize];
                    stream.read_exact(&mut junk).ok();
                }
                paused.store(false, Ordering::Relaxed);
            }
            audio_bridge_proto::FrameKind::Next => {
                // Treat NEXT as “stop accepting control frames for this track/session”.
                if len != 0 {
                    let mut junk = vec![0u8; len as usize];
                    stream.read_exact(&mut junk).ok();
                }
                break;
            }
            audio_bridge_proto::FrameKind::BeginFile => {
                let mut junk = vec![0u8; len as usize];
                stream.read_exact(&mut junk).ok();
                return Err(anyhow!("Unexpected BEGIN_FILE while already receiving a file"));
            }
            audio_bridge_proto::FrameKind::Error => {
                let mut msg = vec![0u8; len as usize];
                stream.read_exact(&mut msg).ok();
                return Err(anyhow!("Sender sent ERROR: {}", String::from_utf8_lossy(&msg)));
            }
        }
    }

    // Ensure done is set even if the connection drops without END_FILE.
    let (lock, cv) = &**progress;
    let mut g = lock.lock().unwrap();
    g.done = true;
    drop(g);
    cv.notify_all();

    Ok(())
}

fn make_temp_path(prefix: &str) -> PathBuf {
    let mut p = std::env::temp_dir();

    // Uniqueness without extra crates.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();

    p.push(format!("{prefix}-{nanos}.bin"));
    p
}

/// A seekable media source backed by a file that is being appended to concurrently.
///
/// Reads will block (via a condition variable) until at least 1 byte is available at the
/// current read position, or the writer marks the stream done.
struct BlockingFileSource {
    file: File,
    progress: Arc<(Mutex<Progress>, Condvar)>,
    pos: u64,
}

impl BlockingFileSource {
    fn new(file: File, progress: Arc<(Mutex<Progress>, Condvar)>) -> Self {
        Self { file, progress, pos: 0 }
    }

    fn wait_until_available(&self, want_pos: u64) {
        let (lock, cv) = &*self.progress;
        let mut g = lock.lock().unwrap();
        while !g.done && g.bytes_written < want_pos {
            g = cv.wait(g).unwrap();
        }
    }
}

impl Read for BlockingFileSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Ensure at least 1 byte is available (or writer is done).
        self.wait_until_available(self.pos.saturating_add(1));

        let (lock, _) = &*self.progress;
        let g = lock.lock().unwrap();

        // True EOF only when writer is done AND we've consumed all written bytes.
        if g.done && self.pos >= g.bytes_written {
            return Ok(0);
        }

        let max_can_read = (g.bytes_written.saturating_sub(self.pos)) as usize;
        let to_read = buf.len().min(max_can_read);
        drop(g);

        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(&mut buf[..to_read])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for BlockingFileSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(x) => x,
            SeekFrom::Current(d) => self.pos.saturating_add_signed(d),
            SeekFrom::End(_) => {
                // For v1: wait until done, then treat end as final length.
                self.wait_until_available(u64::MAX);
                let (lock, _) = &*self.progress;
                let g = lock.lock().unwrap();
                g.bytes_written
            }
        };

        // Block until the seek target exists (or stream is done).
        self.wait_until_available(target);
        self.pos = target;
        Ok(self.pos)
    }
}

impl MediaSource for BlockingFileSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}