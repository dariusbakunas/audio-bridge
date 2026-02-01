//! Scan local directories for playable files (MVP: non-recursive `.flac`/`.wav`).

use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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

/// Best-effort probe for duration + tags.
///
/// Failures are swallowed; unknown fields remain `None`.
pub fn probe_track_meta(path: &Path, ext_hint: &str) -> TrackMeta {
    let mut meta = TrackMeta::default();
    if ext_hint.is_empty() {
        meta.format = None;
    } else {
        meta.format = Some(ext_hint.to_ascii_uppercase());
    }

    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return meta,
    };
    let mut hint = Hint::new();
    if !ext_hint.is_empty() {
        hint.with_extension(ext_hint);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut probed = match symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
    {
        Ok(probed) => probed,
        Err(_) => return meta,
    };

    if let Some(track) = probed.format.default_track() {
        let params = &track.codec_params;
        meta.sample_rate = params.sample_rate;
        if let (Some(frames), Some(rate)) = (params.n_frames, params.sample_rate) {
            if rate > 0 {
                meta.duration_ms = Some(frames.saturating_mul(1000) / rate as u64);
            }
        }
    }

    if let Some(rev) = probed.format.metadata().current() {
        for tag in rev.tags() {
            match tag.std_key {
                Some(symphonia::core::meta::StandardTagKey::Album) => {
                    if meta.album.is_none() {
                        meta.album = Some(tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::Artist) => {
                    if meta.artist.is_none() {
                        meta.artist = Some(tag.value.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    meta
}
