//! Ratatui UI loop.
//!
//! Keys:
//! - Up/Down: move selection
//! - Left/Backspace: go to parent dir
//! - Enter: play selected (or enter dir)
//! - Space: pause/resume
//! - n: next (queue)
//! - k: toggle queue for selected track
//! - K: queue all tracks in current folder
//! - r: rescan directory
//! - l: logs
//! - q: quit

mod app;
mod render;
mod view_model;

pub(crate) use app::run_tui;
