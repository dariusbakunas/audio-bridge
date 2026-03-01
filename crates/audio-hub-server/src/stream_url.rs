//! Shared helpers for constructing stream URLs.

use std::path::PathBuf;

use crate::metadata_db::MetadataDb;
use anyhow::Result;

pub fn build_stream_url_for(
    path: &PathBuf,
    public_base_url: &str,
    metadata: Option<&MetadataDb>,
) -> Result<String> {
    let track_id = metadata
        .ok_or_else(|| anyhow::anyhow!("metadata database is required to build stream url"))?
        .track_id_for_path(&path.to_string_lossy())?
        .ok_or_else(|| anyhow::anyhow!("track id not found for path {}", path.display()))?;
    Ok(format!(
        "{}/stream/track/{track_id}",
        public_base_url.trim_end_matches('/')
    ))
}
