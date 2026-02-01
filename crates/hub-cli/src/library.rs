//! Scan local directories for playable files (MVP: non-recursive `.flac`/`.wav`).

use std::path::{Path, PathBuf};

/// Metadata for a single playable track.
#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub file_name: String,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub format: String,
}

/// Optional metadata collected via Symphonia.
#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub format: Option<String>,
}

/// A directory entry in the file browser list.
#[derive(Clone, Debug)]
pub enum LibraryItem {
    Dir { path: PathBuf, name: String },
    Track(Track),
}

impl LibraryItem {
    pub fn name(&self) -> &str {
        match self {
            LibraryItem::Dir { name, .. } => name,
            LibraryItem::Track(track) => &track.file_name,
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            LibraryItem::Dir { path, .. } => path.as_path(),
            LibraryItem::Track(track) => track.path.as_path(),
        }
    }

    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            LibraryItem::Dir { .. } => None,
            LibraryItem::Track(track) => track.duration_ms,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, LibraryItem::Dir { .. })
    }
}
