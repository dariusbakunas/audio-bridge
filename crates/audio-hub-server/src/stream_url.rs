//! Shared helpers for constructing stream URLs.

use std::path::PathBuf;

use crate::metadata_db::MetadataDb;

pub fn build_stream_url_for(
    path: &PathBuf,
    public_base_url: &str,
    metadata: Option<&MetadataDb>,
) -> String {
    if let Some(track_id) = metadata
        .and_then(|db| db.track_id_for_path(&path.to_string_lossy()).ok().flatten())
    {
        return format!(
            "{}/stream/track/{track_id}",
            public_base_url.trim_end_matches('/')
        );
    }
    let path_str = path.to_string_lossy();
    let encoded = urlencoding::encode(&path_str);
    format!(
        "{}/stream?path={encoded}",
        public_base_url.trim_end_matches('/')
    )
}
