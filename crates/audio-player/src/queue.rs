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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
pub struct SharedAudio {
    channels: usize,
    inner: Mutex<SharedInner>,
    cv: Condvar,
    max_buffered_samples: usize,
    low_watermark_ms: std::sync::atomic::AtomicU64,
}

struct SharedInner {
    queue: VecDeque<f32>,
    done: bool,
}

/// Strategy for popping interleaved frames from the queue.
pub enum PopStrategy {
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
pub fn calc_max_buffered_samples(rate_hz: u32, channels: usize, buffer_seconds: f32) -> usize {
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
    pub fn new(channels: usize, max_buffered_samples: usize) -> Self {
        Self {
            channels,
            inner: Mutex::new(SharedInner {
                queue: VecDeque::new(),
                done: false,
            }),
            cv: Condvar::new(),
            max_buffered_samples,
            low_watermark_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Number of channels for the interleaved sample stream carried by this queue.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Maximum buffered frames (capacity) for this queue.
    pub fn max_frames(&self) -> usize {
        self.max_buffered_samples / self.channels
    }

    /// Current buffered frames (best-effort snapshot).
    ///
    /// This value can change immediately after the call returns.
    pub fn len_frames(&self) -> usize {
        let g = self.inner.lock().unwrap();
        g.queue.len() / self.channels
    }

    /// Whether the queue has been closed by its producer.
    ///
    /// Closed queues may still contain buffered samples until drained.
    pub fn is_done(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.done
    }

    /// Mark the queue as finished and wake all waiters.
    ///
    /// After calling this:
    /// - Blocking pops will eventually return `None` once the queue drains.
    /// - Blocking pushes will stop accepting data and return early.
    ///
    /// This is idempotent and safe to call multiple times.
    pub fn close(&self) {
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
    pub fn push_interleaved_blocking(&self, samples: &[f32]) {
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
    pub fn pop(&self, strategy: PopStrategy) -> Option<Vec<f32>> {
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
                self.log_low_watermark();
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
                self.log_low_watermark();
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
                self.log_low_watermark();
                Some(out)
            }
        }
    }

    fn log_low_watermark(&self) {
        let threshold = (self.max_buffered_samples / 8).max(self.channels * 16);
        let queued = {
            let g = self.inner.lock().unwrap();
            g.queue.len()
        };
        if queued > 0 && queued < threshold {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_millis(0))
                .as_millis() as u64;
            let last = self.low_watermark_ms.load(Ordering::Relaxed);
            if now.saturating_sub(last) > 1000 {
                self.low_watermark_ms.store(now, Ordering::Relaxed);
                tracing::info!(
                    queued_samples = queued,
                    threshold_samples = threshold,
                    "audio queue low watermark"
                );
            }
        }
    }

    /// Wait briefly for any buffered audio to appear.
    ///
    /// Returns `true` if data becomes available before `timeout`.
    pub fn wait_for_any(&self, timeout: Duration) -> bool {
        let mut g = self.inner.lock().unwrap();
        if !g.queue.is_empty() {
            return true;
        }
        let (g2, _timeout) = self.cv.wait_timeout(g, timeout).unwrap();
        g = g2;
        !g.queue.is_empty()
    }
}

/// Block the current thread until `q` is closed and fully drained.
///
/// This is typically used by `main` to wait for background decode/resample stages to finish
/// before exiting the process.
pub fn wait_until_done_and_empty(q: &Arc<SharedAudio>) {
    let mut g = q.inner.lock().unwrap();
    while !(g.done && g.queue.is_empty()) {
        g = q.cv.wait(g).unwrap();
    }
}

/// Block until `q` is closed+empty OR `cancel` becomes true.
///
/// Returns `true` if queue drained normally, `false` if cancelled.
pub fn wait_until_done_and_empty_or_cancel(q: &Arc<SharedAudio>, cancel: &Arc<AtomicBool>) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn calc_max_buffered_samples_fallbacks() {
        assert_eq!(calc_max_buffered_samples(48_000, 2, 2.0), 192_000);
        assert_eq!(calc_max_buffered_samples(48_000, 2, -1.0), 192_000);
        assert_eq!(calc_max_buffered_samples(48_000, 2, f32::NAN), 192_000);
        assert_eq!(calc_max_buffered_samples(48_000, 2, f32::INFINITY), 192_000);
    }

    #[test]
    fn pop_nonblocking_empty() {
        let q = SharedAudio::new(2, 16);
        let out = q.pop(PopStrategy::NonBlocking { max_frames: 4 });
        assert!(out.is_none());
    }

    #[test]
    fn pop_blocking_exact_waits_for_full_frames() {
        let q = Arc::new(SharedAudio::new(2, 64));
        let q_push = q.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let start = barrier.clone();

        let handle = thread::spawn(move || {
            start.wait();
            let out = q.pop(PopStrategy::BlockingExact { frames: 3 }).unwrap();
            assert_eq!(out.len(), 6);
        });

        barrier.wait();
        q_push.push_interleaved_blocking(&[0.1, 0.2, 0.3, 0.4]);
        q_push.push_interleaved_blocking(&[0.5, 0.6]);

        handle.join().unwrap();
    }

    #[test]
    fn pop_blocking_up_to_drains_tail_and_respects_close() {
        let q = Arc::new(SharedAudio::new(2, 64));
        let q_pop = q.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let start = barrier.clone();

        let handle = thread::spawn(move || {
            start.wait();
            let out = q_pop
                .pop(PopStrategy::BlockingUpTo { max_frames: 8 })
                .unwrap();
            assert_eq!(out.len(), 4);
            let out2 = q_pop
                .pop(PopStrategy::BlockingUpTo { max_frames: 8 });
            assert!(out2.is_none());
        });

        barrier.wait();
        q.push_interleaved_blocking(&[1.0, 2.0, 3.0, 4.0]);
        q.close();

        handle.join().unwrap();
    }

    #[test]
    fn pop_nonblocking_returns_available_frames() {
        let q = SharedAudio::new(2, 64);
        q.push_interleaved_blocking(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);

        let out = q.pop(PopStrategy::NonBlocking { max_frames: 2 }).unwrap();
        assert_eq!(out.len(), 4);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn pop_blocking_exact_returns_none_when_closed() {
        let q = SharedAudio::new(2, 64);
        q.close();
        let out = q.pop(PopStrategy::BlockingExact { frames: 1 });
        assert!(out.is_none());
    }

    #[test]
    fn wait_for_any_returns_true_when_data_arrives() {
        let q = Arc::new(SharedAudio::new(2, 64));
        let q_push = q.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = thread::spawn(move || {
            let _ = rx.recv();
            q_push.push_interleaved_blocking(&[1.0, 2.0]);
        });

        let _ = tx.send(());
        assert!(q.wait_for_any(Duration::from_millis(100)));
        handle.join().unwrap();
    }

    #[test]
    fn wait_for_any_returns_false_on_timeout() {
        let q = SharedAudio::new(2, 64);
        assert!(!q.wait_for_any(Duration::from_millis(10)));
    }

    #[test]
    fn wait_until_done_and_empty_returns_when_closed() {
        let q = Arc::new(SharedAudio::new(2, 64));
        q.close();
        wait_until_done_and_empty(&q);
        assert!(q.is_done());
    }

    #[test]
    fn wait_until_done_and_empty_or_cancel_returns_true_when_closed() {
        let q = Arc::new(SharedAudio::new(2, 64));
        let cancel = Arc::new(AtomicBool::new(false));
        q.close();
        let drained = wait_until_done_and_empty_or_cancel(&q, &cancel);
        assert!(drained);
    }

    #[test]
    fn wait_until_done_and_empty_or_cancel_respects_cancel() {
        let q = Arc::new(SharedAudio::new(2, 64));
        let cancel = Arc::new(AtomicBool::new(false));
        cancel.store(true, Ordering::Relaxed);
        let drained = wait_until_done_and_empty_or_cancel(&q, &cancel);
        assert!(!drained);
    }
}
