use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::models::LibraryEntry;

#[derive(Clone, Debug)]
pub struct LibraryIndex {
    root: PathBuf,
    entries_by_dir: std::collections::HashMap<PathBuf, Vec<LibraryEntry>>,
}

impl LibraryIndex {
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    pub fn list_dir(&self, dir: &Path) -> Option<&[LibraryEntry]> {
        self.entries_by_dir.get(dir).map(|v| v.as_slice())
    }

    pub fn find_track_by_path(&self, path: &Path) -> Option<LibraryEntry> {
        let dir = path.parent()?;
        let entries = self.entries_by_dir.get(dir)?;
        let target = path.to_string_lossy();
        for entry in entries {
            if let LibraryEntry::Track { path, .. } = entry {
                if path == target.as_ref() {
                    return Some(entry.clone());
                }
            }
        }
        None
    }
}

pub fn scan_library(root: &Path) -> Result<LibraryIndex> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize root {:?}", root))?;
    if !root.is_dir() {
        return Err(anyhow::anyhow!("root is not a directory: {:?}", root));
    }

    let mut entries_by_dir = std::collections::HashMap::new();
    scan_dir(&root, &root, &mut entries_by_dir)?;

    Ok(LibraryIndex { root, entries_by_dir })
}

fn scan_dir(root: &Path, dir: &Path, entries_by_dir: &mut std::collections::HashMap<PathBuf, Vec<LibraryEntry>>) -> Result<()> {
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
            let path_str = path.to_string_lossy().to_string();
            dirs.push((name.to_lowercase(), LibraryEntry::Dir { path: path_str, name }));
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
        let entry = LibraryEntry::Track {
            path: path.to_string_lossy().to_string(),
            file_name: file_name.clone(),
            ext_hint: ext.clone(),
            duration_ms: meta.duration_ms,
            sample_rate: meta.sample_rate,
            album: meta.album,
            artist: meta.artist,
            format: meta.format.unwrap_or_else(|| "<unknown>".into()),
        };
        tracks.push((file_name.to_lowercase(), entry));
    }

    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    tracks.sort_by(|a, b| a.0.cmp(&b.0));

    let mut entries = Vec::with_capacity(dirs.len() + tracks.len());
    entries.extend(dirs.into_iter().map(|(_, e)| e));
    entries.extend(tracks.into_iter().map(|(_, e)| e));

    entries_by_dir.insert(dir.to_path_buf(), entries);

    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {:?}", dir))? {
        let entry = entry.context("read_dir entry")?;
        let path = entry.path();
        if path.is_dir() {
            let canon = path
                .canonicalize()
                .with_context(|| format!("canonicalize {:?}", path))?;
            if canon.starts_with(root) {
                scan_dir(root, &canon, entries_by_dir)?;
            }
        }
    }

    Ok(())
}

#[derive(Clone, Debug, Default)]
struct TrackMeta {
    duration_ms: Option<u64>,
    sample_rate: Option<u32>,
    album: Option<String>,
    artist: Option<String>,
    format: Option<String>,
}

fn probe_track_meta(path: &Path, ext_hint: &str) -> TrackMeta {
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
    let mut probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
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
