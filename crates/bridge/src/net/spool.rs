//! Temp-file spooling helpers + blocking file reader for Symphonia.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use symphonia::core::io::MediaSource;

/// Shared progress for a spooling temp file (bytes written + done flag).
#[derive(Debug)]
pub(crate) struct Progress {
    pub(crate) bytes_written: u64,
    pub(crate) done: bool,
}

/// Remove stale temp files created by the receiver.
pub(crate) fn cleanup_temp_files(dir: &Path, prefix: &str) -> io::Result<usize> {
    let mut removed = 0usize;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with(prefix) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Generate a unique temp file path without extra dependencies.
pub(crate) fn make_temp_path(dir: &Path, prefix: &str) -> PathBuf {
    let mut p = dir.to_path_buf();

    // Uniqueness without extra crates.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();

    p.push(format!("{prefix}-{nanos}.bin"));
    p
}

pub(crate) fn mark_done(progress: &Arc<(Mutex<Progress>, Condvar)>) {
    let (lock, cv) = &**progress;
    let mut g = lock.lock().unwrap();
    g.done = true;
    drop(g);
    cv.notify_all();
}

pub(crate) fn is_done(progress: &Arc<(Mutex<Progress>, Condvar)>) -> bool {
    let (lock, _) = &**progress;
    let g = lock.lock().unwrap();
    g.done
}

/// A blocking, seekable view of a file that is being appended by another thread.
///
/// This is how Symphonia can probe and decode a stream that's still downloading.
pub(crate) struct BlockingFileSource {
    file: File,
    progress: Arc<(Mutex<Progress>, Condvar)>,
    pos: u64,
}

impl BlockingFileSource {
    pub(crate) fn new(file: File, progress: Arc<(Mutex<Progress>, Condvar)>) -> Self {
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
                // Wait until done, then treat end as final length.
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
