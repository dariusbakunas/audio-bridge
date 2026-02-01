//! Network receiver utilities for “one file per connection” streaming.
//!
//! This module is split into:
//! - `session`: protocol handling and per-connection session management
//! - `spool`: temp-file spooling helpers + blocking reader for Symphonia

use std::path::Path;

mod session;
mod spool;

pub(crate) use session::{accept_one, run_one_client, NetSession};
pub(crate) use spool::BlockingFileSource;

#[derive(Clone, Debug)]
pub(crate) struct DeviceControl {
    pub(crate) selected: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

const TEMP_PREFIX: &str = "audio-bridge-stream";

/// Remove stale temp files created by the receiver.
pub(crate) fn cleanup_temp_files(dir: &Path) -> std::io::Result<usize> {
    spool::cleanup_temp_files(dir, TEMP_PREFIX)
}
