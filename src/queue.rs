use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

pub struct SharedAudio {
    pub channels: usize,
    inner: Mutex<SharedInner>,
    cv: Condvar,
    max_buffered_samples: usize,
}

struct SharedInner {
    queue: VecDeque<f32>,
    done: bool,
}

impl SharedAudio {
    pub fn new(channels: usize, max_buffered_samples: usize) -> Self {
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

    pub fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        g.done = true;
        drop(g);
        self.cv.notify_all();
    }

    pub fn is_done_and_empty(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.done && g.queue.is_empty()
    }

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

    pub fn pop_interleaved_frames_blocking(&self, frames: usize) -> Option<Vec<f32>> {
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

    /// Blocking: pop up to `max_frames` frames (interleaved).
    /// Returns `None` only when `done && empty`.
    pub fn pop_up_to_frames_blocking(&self, max_frames: usize) -> Option<Vec<f32>> {
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

    /// Non-blocking: pop up to `max_frames` frames (interleaved).
    pub fn try_pop_up_to_frames(&self, max_frames: usize) -> Option<Vec<f32>> {
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

pub fn wait_until_done_and_empty(q: &Arc<SharedAudio>) {
    let mut g = q.inner.lock().unwrap();
    while !(g.done && g.queue.is_empty()) {
        g = q.cv.wait(g).unwrap();
    }
}