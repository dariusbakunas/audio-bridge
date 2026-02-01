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

        let duration_ms = probe_duration_ms(&path, &ext);

        tracks.push(LibraryItem::Track(Track {
            path,
            file_name,
            ext_hint: ext,
            duration_ms,
        }));
    }

    dirs.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));
    tracks.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));

    let mut out = Vec::with_capacity(dirs.len() + tracks.len());
    out.extend(dirs);
    out.extend(tracks);
    Ok(out)
}

fn probe_duration_ms(path: &Path, ext_hint: &str) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut hint = Hint::new();
    if !ext_hint.is_empty() {
        hint.with_extension(ext_hint);
    }

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;

    let track = probed.format.default_track()?;
    let params = &track.codec_params;
    let frames = params.n_frames?;
    let rate = params.sample_rate? as u64;
    if rate == 0 {
        return None;
    }
    Some(frames.saturating_mul(1000) / rate)
}
