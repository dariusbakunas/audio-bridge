use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use symphonia::core::io::MediaSource;

#[derive(Clone, Debug)]
pub(crate) struct HttpRangeConfig {
    pub(crate) block_size: usize,
    pub(crate) timeout: Duration,
}

impl Default for HttpRangeConfig {
    fn default() -> Self {
        Self {
            block_size: 512 * 1024,
            timeout: Duration::from_secs(10),
        }
    }
}

/// A simple HTTP range reader with a small in-memory block cache.
pub(crate) struct HttpRangeSource {
    url: String,
    config: HttpRangeConfig,
    pos: u64,
    len: Option<u64>,
    buf: Vec<u8>,
    buf_start: u64,
    cancel: Option<Arc<AtomicBool>>,
}

impl HttpRangeSource {
    pub(crate) fn new(
        url: String,
        config: HttpRangeConfig,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            url,
            config,
            pos: 0,
            len: None,
            buf: Vec::new(),
            buf_start: 0,
            cancel,
        }
    }

    fn is_canceled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn ensure_len(&mut self) -> io::Result<u64> {
        if let Some(len) = self.len {
            return Ok(len);
        }
        let (data, len) = self.fetch_range(0, 0)?;
        let len = len.ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "content length unavailable")
        })?;
        self.buf_start = 0;
        self.buf = data;
        self.len = Some(len);
        Ok(len)
    }

    fn fetch_range(&self, start: u64, end: u64) -> io::Result<(Vec<u8>, Option<u64>)> {
        let range = format!("bytes={start}-{end}");
        let start = std::time::Instant::now();
        let resp = ureq::get(&self.url)
            .config()
            .timeout_per_call(Some(self.config.timeout))
            .build()
            .header("Range", &range)
            .call()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("http range request failed: {e}")))?;
        let elapsed = start.elapsed();

        let status = resp.status();
        let content_range = resp
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_length = resp
            .headers()
            .get("Content-Length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let mut buf = Vec::new();
        let (_, body) = resp.into_parts();
        body.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("http read failed: {e}")))?;
        if elapsed > Duration::from_millis(250) {
            let kbps = if elapsed.as_millis() > 0 {
                (buf.len() as u128 * 1000 / elapsed.as_millis()) / 1024
            } else {
                0
            };
            tracing::warn!(
                took_ms = elapsed.as_millis(),
                bytes = buf.len(),
                kbps = kbps as u64,
                range = range.as_str(),
                "http range fetch slow"
            );
        }

        let len = match status {
            ureq::http::StatusCode::PARTIAL_CONTENT => content_range
                .as_deref()
                .and_then(parse_content_range_total)
                .or(content_length),
            ureq::http::StatusCode::OK => content_length,
            _ => None,
        };

        Ok((buf, len))
    }

    fn refill(&mut self) -> io::Result<()> {
        if self.is_canceled() {
            return Ok(());
        }

        let start = self.pos;
        let mut end = start.saturating_add(self.config.block_size as u64).saturating_sub(1);
        if let Some(len) = self.len {
            if len > 0 {
                end = end.min(len.saturating_sub(1));
            }
        }

        let (buf, len) = self.fetch_range(start, end)?;
        if let Some(total) = len {
            self.len = Some(total);
        }
        self.buf = buf;
        self.buf_start = start;
        Ok(())
    }
}

impl Read for HttpRangeSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.is_canceled() {
            return Ok(0);
        }
        if out.is_empty() {
            return Ok(0);
        }
        if let Some(len) = self.len {
            if self.pos >= len {
                return Ok(0);
            }
        }

        if self.buf.is_empty()
            || self.pos < self.buf_start
            || self.pos >= self.buf_start.saturating_add(self.buf.len() as u64)
        {
            self.refill()?;
        }

        if self.buf.is_empty() {
            return Ok(0);
        }

        let offset = (self.pos.saturating_sub(self.buf_start)) as usize;
        if offset >= self.buf.len() {
            return Ok(0);
        }

        let available = self.buf.len().saturating_sub(offset);
        let to_copy = available.min(out.len());
        out[..to_copy].copy_from_slice(&self.buf[offset..offset + to_copy]);
        self.pos = self.pos.saturating_add(to_copy as u64);
        Ok(to_copy)
    }
}

impl Seek for HttpRangeSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(x) => x,
            SeekFrom::Current(d) => add_signed(self.pos, d),
            SeekFrom::End(d) => {
                let len = self.ensure_len()?;
                add_signed(len, d)
            }
        };
        self.pos = target;
        Ok(self.pos)
    }
}

impl MediaSource for HttpRangeSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

fn parse_content_range_total(header: &str) -> Option<u64> {
    // Format: "bytes start-end/total"
    let (_, total) = header.split_once('/')?;
    total.parse::<u64>().ok()
}

fn add_signed(base: u64, delta: i64) -> u64 {
    if delta >= 0 {
        base.saturating_add(delta as u64)
    } else {
        let neg = delta.checked_abs().unwrap_or(i64::MAX) as u64;
        base.saturating_sub(neg)
    }
}
