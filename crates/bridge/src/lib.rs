//! Bridge crate entry points.
//!
//! Provides CLI parsing, configuration, and runtime helpers for the bridge.

pub mod cli;
pub mod config;
pub mod runtime;

mod http_api;
mod http_stream;
mod mdns;
mod player;
mod status;
