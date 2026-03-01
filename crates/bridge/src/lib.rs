//! Bridge crate entry points.
//!
//! Provides CLI parsing, configuration, and runtime helpers for the bridge.

/// Command-line argument definitions.
pub mod cli;
/// Runtime configuration types for listen/play modes.
pub mod config;
/// Top-level execution helpers for bridge commands.
pub mod runtime;

mod exclusive;
mod http_api;
mod http_stream;
mod mdns;
mod player;
mod status;
