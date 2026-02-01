//! Ratatui UI loop.
//!
//! Keys:
//! - Up/Down: move selection
//! - Left/Backspace: go to parent dir
//! - Enter: play selected (or enter dir)
//! - Space: pause/resume
//! - n: next (immediate skip)
//! - r: rescan directory
//! - q: quit

mod app;
mod render;

pub(crate) use app::run_tui;
