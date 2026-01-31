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
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use symphonia::core::io::MediaSource;
use symphonia::core::probe::Hint;

const MAGIC: &[u8; 4] = b"ABRD";
const VERSION: u16 = 1;

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

/// Receive one streamed file from `stream`, spooling it to a temp file.
///
/// Returns:
/// - A Symphonia [`Hint`] derived from the sent extension.
/// - The temp file path.
/// - A boxed [`MediaSource`] that blocks until bytes are available (and is seekable).
pub(crate) fn recv_one_file_as_media_source(
    mut stream: TcpStream,
) -> Result<(IncomingStreamInfo, Box<dyn MediaSource>)> {
    let hint = read_header_make_hint(&mut stream)?;

    let temp_path = make_temp_path("audio-bridge-stream");
    let writer_path = temp_path.clone();

    // Create/truncate temp file up front.
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

    // Writer thread: read from socket, append to temp file, signal readers.
    let progress_writer = progress.clone();
    thread::spawn(move || {
        if let Err(e) = writer_thread_main(&mut stream, &writer_path, &progress_writer) {
            eprintln!("Network writer error: {e:#}");
        }
        // Ensure readers eventually stop waiting even on error.
        let (lock, cv) = &*progress_writer;
        let mut g = lock.lock().unwrap();
        g.done = true;
        drop(g);
        cv.notify_all();
    });

    // Reader handle for Symphonia.
    let file_for_read = OpenOptions::new()
        .read(true)
        .open(&temp_path)
        .with_context(|| format!("open temp file for read {:?}", temp_path))?;

    let source = Box::new(BlockingFileSource::new(file_for_read, progress.clone()));

    Ok((
        IncomingStreamInfo {
            hint,
            temp_path,
        },
        source,
    ))
}

/// Read the simple header and convert it into a Symphonia [`Hint`].
fn read_header_make_hint(stream: &mut TcpStream) -> Result<Hint> {
    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).context("read magic")?;
    if &magic != MAGIC {
        return Err(anyhow!("Bad magic (expected {:?})", MAGIC));
    }

    let version = read_u16_le(stream).context("read version")?;
    if version != VERSION {
        return Err(anyhow!("Unsupported protocol version {version}"));
    }

    let ext_len = read_u16_le(stream).context("read ext_len")? as usize;
    let mut ext_bytes = vec![0u8; ext_len];
    stream
        .read_exact(&mut ext_bytes)
        .context("read extension bytes")?;

    let ext = std::str::from_utf8(&ext_bytes).context("extension not utf-8")?;
    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }
    Ok(hint)
}

fn read_u16_le(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
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

fn writer_thread_main(
    stream: &mut TcpStream,
    path: &PathBuf,
    progress: &Arc<(Mutex<Progress>, Condvar)>,
) -> Result<()> {
    let mut f = OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("open temp file for append {:?}", path))?;

    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break, // EOF: one file per connection
            Ok(n) => n,
            Err(e) => return Err(e).context("read from socket"),
        };

        f.write_all(&buf[..n]).context("write to temp file")?;
        f.flush().ok(); // best-effort

        let (lock, cv) = &**progress;
        let mut g = lock.lock().unwrap();
        g.bytes_written = g.bytes_written.saturating_add(n as u64);
        drop(g);
        cv.notify_all();
    }

    let (lock, cv) = &**progress;
    let mut g = lock.lock().unwrap();
    g.done = true;
    drop(g);
    cv.notify_all();

    Ok(())
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