//! Scan local directories for playable files (MVP: non-recursive `.flac`/`.wav`).

use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub file_name: String,
    pub ext_hint: String,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub format: String,
}

#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub format: Option<String>,
}

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

pub fn list_entries(dir: &Path) -> Result<Vec<LibraryItem>> {
    let mut dirs = Vec::new();
    let mut tracks = Vec::new();

    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {:?}", dir))? {
        let entry = entry.context("read_dir entry")?;
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("<unknown>")
                .to_string();
            dirs.push(LibraryItem::Dir { path, name });
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();

        if ext != "flac" && ext != "wav" {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>")
            .to_string();

        let meta = probe_track_meta(&path, &ext);

        tracks.push(LibraryItem::Track(Track {
            path,
            file_name,
            ext_hint: ext,
            duration_ms: meta.duration_ms,
            sample_rate: meta.sample_rate,
            album: meta.album,
            artist: meta.artist,
            format: meta.format.unwrap_or_else(|| "<unknown>".into()),
        }));
    }

    dirs.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));
    tracks.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));

    let mut out = Vec::with_capacity(dirs.len() + tracks.len());
    out.extend(dirs);
    out.extend(tracks);
    Ok(out)
}

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
