//! HTTP range reader used for streaming playback.
//!
//! Implements a simple buffered range fetcher over HTTP.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use symphonia::core::io::MediaSource;

/// Configuration for HTTP range fetching.
#[derive(Clone, Debug)]
pub(crate) struct HttpRangeConfig {
    /// Bytes per fetched block.
    pub(crate) block_size: usize,
    /// Per-request timeout.
    pub(crate) timeout: Duration,
    /// Number of retry attempts for transient range failures.
    pub(crate) retry_attempts: usize,
    /// Base backoff for retries (multiplied by attempt number).
    pub(crate) retry_backoff: Duration,
    /// Allow insecure TLS (self-signed certs).
    pub(crate) tls_insecure: bool,
}

impl Default for HttpRangeConfig {
    fn default() -> Self {
        Self {
            block_size: 512 * 1024,
            timeout: Duration::from_secs(10),
            retry_attempts: 5,
            retry_backoff: Duration::from_millis(200),
            tls_insecure: false,
        }
    }
}

/// A simple HTTP range reader with a small in-memory block cache.
pub(crate) struct HttpRangeSource {
    url: String,
    config: HttpRangeConfig,
    agent: ureq::Agent,
    pos: u64,
    len: Option<u64>,
    buf: Vec<u8>,
    buf_start: u64,
    cancel: Option<Arc<AtomicBool>>,
    error_flag: Option<Arc<AtomicBool>>,
}

impl HttpRangeSource {
    /// Create a new range source for a URL with optional cancel flag.
    pub(crate) fn new(
        url: String,
        config: HttpRangeConfig,
        cancel: Option<Arc<AtomicBool>>,
        error_flag: Option<Arc<AtomicBool>>,
    ) -> Self {
        let agent = build_agent(config.tls_insecure);
        Self {
            url,
            config,
            agent,
            pos: 0,
            len: None,
            buf: Vec::new(),
            buf_start: 0,
            cancel,
            error_flag,
        }
    }

    fn is_canceled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn mark_error(&self) {
        if let Some(flag) = &self.error_flag {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Ensure the total length is known by issuing a range probe.
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

    /// Fetch a byte range from the remote server.
    fn fetch_range(&self, start: u64, end: u64) -> io::Result<(Vec<u8>, Option<u64>)> {
        let range = format!("bytes={start}-{end}");
        let mut last_err: Option<io::Error> = None;
        let attempts = self.config.retry_attempts.max(1);
        for attempt in 1..=attempts {
            let started = std::time::Instant::now();
            tracing::debug!(
                url = %self.url,
                range = %range,
                attempt,
                "http range request"
            );
            let resp = self.agent.get(&self.url)
                .config()
                .timeout_per_call(Some(self.config.timeout))
                .build()
                .header("Range", &range)
                .call();
            let mut resp = match resp {
                Ok(resp) => resp,
                Err(e) => {
                    let err = io::Error::new(
                        io::ErrorKind::Other,
                        format!("http range request failed: {e}"),
                    );
                    last_err = Some(err);
                    if attempt < attempts {
                        let backoff = self.config.retry_backoff.saturating_mul(attempt as u32);
                        tracing::warn!(
                            url = %self.url,
                            range = %range,
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            "http range request failed; retrying"
                        );
                        std::thread::sleep(backoff);
                        continue;
                    }
                    self.mark_error();
                    tracing::error!(
                        url = %self.url,
                        range = %range,
                        "http range request failed: {e}"
                    );
                    return Err(last_err.unwrap());
                }
            };
            let elapsed = started.elapsed();

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
            let content_type = resp
                .headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            tracing::debug!(
                status = ?status,
                url = %self.url,
                range = %range,
                content_length = ?content_length,
                content_range = ?content_range,
                content_type = ?content_type,
                "http range response headers"
            );

            let mut buf = Vec::new();
            let (_, body) = resp.into_parts();
            if let Err(e) = body.into_reader().read_to_end(&mut buf) {
                last_err = Some(io::Error::new(
                    io::ErrorKind::Other,
                    format!("http read failed: {e}"),
                ));
                if attempt < attempts {
                    let backoff = self.config.retry_backoff.saturating_mul(attempt as u32);
                    tracing::warn!(
                        url = %self.url,
                        range = %range,
                        attempt,
                        backoff_ms = backoff.as_millis(),
                        "http read failed; retrying"
                    );
                    std::thread::sleep(backoff);
                    continue;
                }
                self.mark_error();
                return Err(last_err.unwrap());
            }
            tracing::debug!(
                status = ?status,
                url = %self.url,
                range = %range,
                bytes = buf.len(),
                content_length = ?content_length,
                content_range = ?content_range,
                content_type = ?content_type,
                "http range fetch"
            );
            if status != ureq::http::StatusCode::OK
                && status != ureq::http::StatusCode::PARTIAL_CONTENT
            {
                let snippet = String::from_utf8_lossy(&buf);
                let snippet = snippet.trim();
                let detail = if snippet.is_empty() {
                    String::from("")
                } else {
                    format!(" body=\"{}\"", snippet)
                };
                last_err = Some(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "http range status={status} url={} range={} content_length={:?} content_range={:?} content_type={:?}{}",
                        self.url, range, content_length, content_range, content_type, detail
                    ),
                ));
                if attempt < attempts && status.is_server_error() {
                    let backoff = self.config.retry_backoff.saturating_mul(attempt as u32);
                    tracing::warn!(
                        url = %self.url,
                        range = %range,
                        status = ?status,
                        attempt,
                        backoff_ms = backoff.as_millis(),
                        "http range status error; retrying"
                    );
                    std::thread::sleep(backoff);
                    continue;
                }
                self.mark_error();
                return Err(last_err.unwrap());
            }
            if buf.is_empty() {
                last_err = Some(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "http range empty body status={status} url={} range={} content_length={:?} content_range={:?} content_type={:?}",
                        self.url, range, content_length, content_range, content_type
                    ),
                ));
                if attempt < attempts {
                    let backoff = self.config.retry_backoff.saturating_mul(attempt as u32);
                    tracing::warn!(
                        url = %self.url,
                        range = %range,
                        attempt,
                        backoff_ms = backoff.as_millis(),
                        "http range empty body; retrying"
                    );
                    std::thread::sleep(backoff);
                    continue;
                }
                self.mark_error();
                return Err(last_err.unwrap());
            }
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

            return Ok((buf, len));
        }
        self.mark_error();
        Err(last_err.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "http range request failed")
        }))
    }

    /// Fill the in-memory buffer starting at the current position.
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
        tracing::debug!(
            url = %self.url,
            start = start,
            end = end,
            bytes = buf.len(),
            total_len = ?len,
            "http range refill"
        );
        if let Some(total) = len {
            self.len = Some(total);
        }
        self.buf = buf;
        self.buf_start = start;
        Ok(())
    }
}

fn build_agent(tls_insecure: bool) -> ureq::Agent {
    let mut tls_builder = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier);
    if tls_insecure {
        tls_builder = tls_builder.disable_verification(true);
    }
    let tls = tls_builder.build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .build()
        .new_agent()
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
                tracing::debug!(
                    url = %self.url,
                    pos = self.pos,
                    len = len,
                    "http read reached end"
                );
                return Ok(0);
            }
        }

        if self.buf.is_empty()
            || self.pos < self.buf_start
            || self.pos >= self.buf_start.saturating_add(self.buf.len() as u64)
        {
            if let Err(err) = self.refill() {
                self.mark_error();
                return Err(err);
            }
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

/// Extract the total length from a Content-Range header.
fn parse_content_range_total(header: &str) -> Option<u64> {
    // Format: "bytes start-end/total"
    let (_, total) = header.split_once('/')?;
    total.parse::<u64>().ok()
}

/// Add a signed delta to an unsigned base with saturation.
fn add_signed(base: u64, delta: i64) -> u64 {
    if delta >= 0 {
        base.saturating_add(delta as u64)
    } else {
        let neg = delta.checked_abs().unwrap_or(i64::MAX) as u64;
        base.saturating_sub(neg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};

    #[test]
    fn default_config_has_expected_values() {
        let cfg = HttpRangeConfig::default();
        assert_eq!(cfg.block_size, 512 * 1024);
        assert_eq!(cfg.timeout, Duration::from_secs(10));
    }

    #[test]
    fn new_source_initializes_empty_buffer() {
        let cfg = HttpRangeConfig::default();
        let source = HttpRangeSource::new(
            "http://example/track.flac".to_string(),
            cfg,
            None,
            None,
        );
        assert_eq!(source.pos, 0);
        assert!(source.len.is_none());
        assert!(source.buf.is_empty());
        assert_eq!(source.buf_start, 0);
    }

    #[test]
    fn add_signed_saturates_on_overflow() {
        assert_eq!(add_signed(u64::MAX, 10), u64::MAX);
    }

    #[test]
    fn parse_content_range_total_reads_total() {
        let total = parse_content_range_total("bytes 0-99/12345");
        assert_eq!(total, Some(12345));
    }

    #[test]
    fn parse_content_range_total_rejects_invalid() {
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(parse_content_range_total("invalid"), None);
    }

    #[test]
    fn parse_content_range_total_requires_slash() {
        assert_eq!(parse_content_range_total("bytes 0-99"), None);
    }

    #[test]
    fn read_reads_from_buffer_and_advances() {
        let cfg = HttpRangeConfig::default();
        let mut source = HttpRangeSource::new(
            "http://example/track.flac".to_string(),
            cfg,
            None,
            None,
        );
        source.len = Some(4);
        source.buf_start = 0;
        source.buf = vec![1, 2, 3, 4];
        source.pos = 1;

        let mut out = [0u8; 2];
        let read = source.read(&mut out).unwrap();
        assert_eq!(read, 2);
        assert_eq!(out, [2, 3]);
        assert_eq!(source.pos, 3);
    }

    #[test]
    fn read_returns_zero_when_canceled() {
        let cfg = HttpRangeConfig::default();
        let cancel = Arc::new(AtomicBool::new(true));
        let mut source = HttpRangeSource::new(
            "http://example/track.flac".to_string(),
            cfg,
            Some(cancel),
            None,
        );
        let mut out = [0u8; 4];
        let read = source.read(&mut out).unwrap();
        assert_eq!(read, 0);
    }

    #[test]
    fn seek_start_sets_position() {
        let cfg = HttpRangeConfig::default();
        let mut source = HttpRangeSource::new(
            "http://example/track.flac".to_string(),
            cfg,
            None,
            None,
        );
        let pos = source.seek(SeekFrom::Start(5)).unwrap();
        assert_eq!(pos, 5);
        assert_eq!(source.pos, 5);
    }

    #[test]
    fn seek_current_allows_negative() {
        let cfg = HttpRangeConfig::default();
        let mut source = HttpRangeSource::new(
            "http://example/track.flac".to_string(),
            cfg,
            None,
            None,
        );
        source.pos = 5;
        let pos = source.seek(SeekFrom::Current(-3)).unwrap();
        assert_eq!(pos, 2);
        assert_eq!(source.pos, 2);
    }

    #[test]
    fn add_signed_handles_positive_and_negative() {
        assert_eq!(add_signed(10, 5), 15);
        assert_eq!(add_signed(10, -3), 7);
    }

    #[test]
    fn add_signed_saturates_on_underflow() {
        assert_eq!(add_signed(5, -10), 0);
    }
}
