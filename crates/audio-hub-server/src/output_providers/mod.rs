//! Output provider implementations and registry wiring.
//!
//! Includes bridge-backed and local providers plus the shared registry.

pub(crate) mod bridge_provider;
pub(crate) mod browser_provider;
pub(crate) mod local_provider;
pub(crate) mod registry;
