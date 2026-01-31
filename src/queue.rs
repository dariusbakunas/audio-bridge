//! Thread-safe bounded queues for interleaved audio samples.
//!
//! The rest of the crate uses [`SharedAudio`] as the “wire format” between stages:
//! - decode thread → queue
//! - resampler thread → queue
//! - CPAL callback drains queue (non-blocking)
//!
//! The API is designed to make shutdown deterministic (`close()` + draining semantics)
//! while keeping the playback callback real-time friendly.


use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Thread-safe bounded queue for interleaved `f32` audio samples.
///
/// ## Design
/// - **Multiple producers / multiple consumers**: safe to call from many threads.
/// - **Bounded** by `max_buffered_samples` to cap memory and latency.
/// - Uses a single [`Condvar`] as a general “state changed” signal.
/// - A `done` flag is stored *under the same mutex* as the queue to avoid races.
///
/// ## Data model
/// Samples are stored **interleaved**:
/// `frame0[ch0], frame0[ch1], ..., frame1[ch0], frame1[ch1], ...`
///
/// The `channels` count is fixed for the lifetime of the queue.
pub(crate) struct SharedAudio {
    channels: usize,
    inner: Mutex<SharedInner>,
    cv: Condvar,
    max_buffered_samples: usize,
}

struct SharedInner {
    queue: VecDeque<f32>,
    done: bool,
}

/// Strategy for popping interleaved frames from the queue.
pub(crate) enum PopStrategy {
    /// Block until exactly `frames` are available, or return `None` if closed before enough data.
    BlockingExact { frames: usize },
    /// Block until at least one frame is available, then return up to `max_frames`.
    BlockingUpTo { max_frames: usize },
    /// Return immediately with up to `max_frames`, or `None` if currently empty.
    NonBlocking { max_frames: usize },
}

/// Compute a conservative queue capacity in **samples** for a `(rate, channels, seconds)` target.
///
/// This is used to size bounded queues for decode/resample stages.
///
/// - If `buffer_seconds` is non-finite or `<= 0.0`, a safe fallback is used.
/// - The returned value is `ceil(rate_hz * buffer_seconds) * channels` (saturating).
pub(crate) fn calc_max_buffered_samples(rate_hz: u32, channels: usize, buffer_seconds: f32) -> usize {
    let secs = if buffer_seconds.is_finite() && buffer_seconds > 0.0 {
        buffer_seconds
    } else {
        2.0
    };

    let frames = (rate_hz as f32 * secs).ceil() as usize;
    frames.saturating_mul(channels)
}

impl SharedAudio {
    /// Create a new bounded queue.
    ///
    /// `max_buffered_samples` is a cap in **samples** (not frames). If you want “N seconds
    /// of audio”, prefer using [`calc_max_buffered_samples`].
    pub(crate) fn new(channels: usize, max_buffered_samples: usize) -> Self {
        Self {
            channels,
            inner: Mutex::new(SharedInner {
                queue: VecDeque::new(),
                done: false,
            }),
            cv: Condvar::new(),
            max_buffered_samples,
        }
    }

    /// Number of channels for the interleaved sample stream carried by this queue.
    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    /// Mark the queue as finished and wake all waiters.
    ///
    /// After calling this:
    /// - Blocking pops will eventually return `None` once the queue drains.
    /// - Blocking pushes will stop accepting data and return early.
    ///
    /// This is idempotent and safe to call multiple times.
    pub(crate) fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.done = true;
        drop(g);
        self.cv.notify_all();
    }

    /// Push interleaved samples into the queue, blocking when the queue is full.
    ///
    /// - Blocks until enough capacity is available, unless the queue is closed.
    /// - If the queue is closed while waiting, this returns early and drops remaining samples.
    ///
    /// Callers should push whole frames when possible, but this method accepts any slice length.
    pub(crate) fn push_interleaved_blocking(&self, samples: &[f32]) {
        let mut offset = 0;

        while offset < samples.len() {
            let mut g = self.inner.lock().unwrap();

            while g.queue.len() >= self.max_buffered_samples && !g.done {
                g = self.cv.wait(g).unwrap();
            }
            if g.done {
                return;
            }

            let mut pushed_any = false;
            while offset < samples.len() && g.queue.len() < self.max_buffered_samples {
                g.queue.push_back(samples[offset]);
                offset += 1;
                pushed_any = true;
            }

            drop(g);
            if pushed_any {
                self.cv.notify_all();
            }
        }
    }

    /// Pop interleaved frames using the requested strategy.
    ///
    /// Returns `None` when the queue is closed and no data can satisfy the request.
    pub(crate) fn pop(&self, strategy: PopStrategy) -> Option<Vec<f32>> {
        match strategy {
            PopStrategy::BlockingExact { frames } => {
                let want = frames * self.channels;
                let mut g = self.inner.lock().unwrap();

                while g.queue.len() < want && !g.done {
                    g = self.cv.wait(g).unwrap();
                }

                if g.queue.len() < want {
                    return None;
                }

                let mut out = Vec::with_capacity(want);
                for _ in 0..want {
                    out.push(g.queue.pop_front().unwrap_or(0.0));
                }

                drop(g);
                self.cv.notify_all();
                Some(out)
            }
            PopStrategy::BlockingUpTo { max_frames } => {
                let mut g = self.inner.lock().unwrap();

                while g.queue.is_empty() && !g.done {
                    g = self.cv.wait(g).unwrap();
                }

                if g.queue.is_empty() && g.done {
                    return None;
                }

                let available_frames = g.queue.len() / self.channels;
                let take_frames = available_frames.min(max_frames);
                let take_samples = take_frames * self.channels;

                let mut out = Vec::with_capacity(take_samples);
                for _ in 0..take_samples {
                    out.push(g.queue.pop_front().unwrap_or(0.0));
                }

                drop(g);
                self.cv.notify_all();
                Some(out)
            }
            PopStrategy::NonBlocking { max_frames } => {
                let mut g = self.inner.lock().unwrap();

                let available_frames = g.queue.len() / self.channels;
                let take_frames = available_frames.min(max_frames);
                let take_samples = take_frames * self.channels;

                if take_samples == 0 {
                    return None;
                }

                let mut out = Vec::with_capacity(take_samples);
                for _ in 0..take_samples {
                    out.push(g.queue.pop_front().unwrap_or(0.0));
                }

                drop(g);
                self.cv.notify_all();
                Some(out)
            }
        }
    }
}

/// Block the current thread until `q` is closed and fully drained.
///
/// This is typically used by `main` to wait for background decode/resample stages to finish
/// before exiting the process.
pub(crate) fn wait_until_done_and_empty(q: &Arc<SharedAudio>) {
    let mut g = q.inner.lock().unwrap();
    while !(g.done && g.queue.is_empty()) {
        g = q.cv.wait(g).unwrap();
    }
}

/// Block until `q` is closed+empty OR `cancel` becomes true.
///
/// Returns `true` if queue drained normally, `false` if cancelled.
pub(crate) fn wait_until_done_and_empty_or_cancel(q: &Arc<SharedAudio>, cancel: &Arc<AtomicBool>) -> bool {
    let mut g = q.inner.lock().unwrap();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }

        if g.done && g.queue.is_empty() {
            return true;
        }

        let (ng, _timeout) = q
            .cv
            .wait_timeout(g, Duration::from_millis(50))
            .unwrap();
        g = ng;
    }
}
