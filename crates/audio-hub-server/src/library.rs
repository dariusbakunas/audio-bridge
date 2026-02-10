//! Library scanning and indexing.
//!
//! Walks the media root, extracts metadata, and builds lookup maps.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardVisualKey};
use symphonia::core::probe::Hint;

use crate::models::LibraryEntry;

/// In-memory index of the media library rooted at a directory.
#[derive(Clone, Debug)]
pub struct LibraryIndex {
    root: PathBuf,
    entries_by_dir: std::collections::HashMap<PathBuf, Vec<LibraryEntry>>,
}

impl LibraryIndex {
    /// Return the canonical library root path.
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    /// List entries for a directory in the library.
    pub fn list_dir(&self, dir: &Path) -> Option<&[LibraryEntry]> {
        self.entries_by_dir.get(dir).map(|v| v.as_slice())
    }

    /// Find a specific track entry by absolute path.
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

    pub fn update_track_meta(&mut self, path: &Path, meta: &TrackMeta) -> bool {
        let dir = match path.parent() {
            Some(dir) => dir,
            None => return false,
        };
        let Some(entries) = self.entries_by_dir.get_mut(dir) else {
            return false;
        };
        let path_str = path.to_string_lossy();
        let ext_hint = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        for entry in entries.iter_mut() {
            if let LibraryEntry::Track { path, ext_hint: hint, duration_ms, sample_rate, album, artist, format, .. } = entry {
                if path == path_str.as_ref() {
                    *duration_ms = meta.duration_ms;
                    *sample_rate = meta.sample_rate;
                    *album = meta.album.clone();
                    *artist = meta.artist.clone();
                    *format = meta.format.clone().unwrap_or_else(|| ext_hint.clone());
                    *hint = meta.format.clone().unwrap_or_else(|| ext_hint.clone());
                    return true;
                }
            }
        }
        false
    }

    pub fn remove_track(&mut self, path: &Path) -> bool {
        let dir = match path.parent() {
            Some(dir) => dir,
            None => return false,
        };
        let Some(entries) = self.entries_by_dir.get_mut(dir) else {
            return false;
        };
        let path_str = path.to_string_lossy();
        let before = entries.len();
        entries.retain(|entry| {
            if let LibraryEntry::Track { path, .. } = entry {
                path != path_str.as_ref()
            } else {
                true
            }
        });
        before != entries.len()
    }
}

/// Scan the media root and build a new library index.
pub fn scan_library(root: &Path) -> Result<LibraryIndex> {
    scan_library_with_meta(
        root,
        |_path, _file_name, _ext, _meta, _fs_meta| {},
        |_dir, _count| {},
    )
}

/// Scan the media root and build a new library index, invoking `on_track` per file.
pub fn scan_library_with_meta<F, D>(
    root: &Path,
    mut on_track: F,
    mut on_dir: D,
) -> Result<LibraryIndex>
where
    F: FnMut(&Path, &str, &str, &TrackMeta, &std::fs::Metadata),
    D: FnMut(&Path, usize),
{
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize root {:?}", root))?;
    if !root.is_dir() {
        return Err(anyhow::anyhow!("root is not a directory: {:?}", root));
    }

    tracing::info!(root = %root.display(), "scanning library");

    let mut entries_by_dir = std::collections::HashMap::new();
    scan_dir(&root, &root, &mut entries_by_dir, &mut on_track, &mut on_dir)?;

    tracing::info!(root = %root.display(), dirs = entries_by_dir.len(), "library scan complete");
    Ok(LibraryIndex { root, entries_by_dir })
}

fn scan_dir<F, D>(
    root: &Path,
    dir: &Path,
    entries_by_dir: &mut std::collections::HashMap<PathBuf, Vec<LibraryEntry>>,
    on_track: &mut F,
    on_dir: &mut D,
) -> Result<()>
where
    F: FnMut(&Path, &str, &str, &TrackMeta, &std::fs::Metadata),
    D: FnMut(&Path, usize),
{
    let mut dirs = Vec::new();
    let mut tracks = Vec::new();
    let mut has_tracks = false;

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
        if !is_supported_extension(&ext) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>")
            .to_string();

        let meta = probe_track_meta(&path, &ext);
        let fs_meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if !has_tracks {
            has_tracks = true;
            on_dir(dir, 0);
        }
        on_track(&path, &file_name, &ext, &meta, &fs_meta);
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

    let track_count = tracks.len();
    let mut entries = Vec::with_capacity(dirs.len() + tracks.len());
    entries.extend(dirs.into_iter().map(|(_, e)| e));
    entries.extend(tracks.into_iter().map(|(_, e)| e));

    entries_by_dir.insert(dir.to_path_buf(), entries);
    if has_tracks {
        on_dir(dir, track_count);
    }

    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {:?}", dir))? {
        let entry = entry.context("read_dir entry")?;
        let path = entry.path();
        if path.is_dir() {
            let canon = path
                .canonicalize()
                .with_context(|| format!("canonicalize {:?}", path))?;
            if canon.starts_with(root) {
                scan_dir(root, &canon, entries_by_dir, on_track, on_dir)?;
            }
        }
    }

    Ok(())
}

fn is_supported_extension(ext: &str) -> bool {
    matches!(
        ext,
        "flac" | "wav" | "aiff" | "aif" | "mp3" | "m4a" | "aac" | "alac" | "ogg" | "oga" | "opus"
    )
}

#[derive(Clone, Debug, Default)]
pub struct TrackMeta {
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub compilation: bool,
    pub title: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub format: Option<String>,
    pub cover_art: Option<CoverArt>,
}

#[derive(Clone, Debug)]
pub struct CoverArt {
    pub mime_type: String,
    pub data: Vec<u8>,
}

const MAX_COVER_ART_BYTES: usize = 5_000_000;

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
        meta.bit_depth = params.bits_per_sample;
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
                Some(symphonia::core::meta::StandardTagKey::AlbumArtist) => {
                    if meta.album_artist.is_none() {
                        meta.album_artist = Some(tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::Compilation) => {
                    if !meta.compilation {
                        meta.compilation = parse_bool_tag(&tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::Artist) => {
                    if meta.artist.is_none() {
                        meta.artist = Some(tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::TrackTitle) => {
                    if meta.title.is_none() {
                        meta.title = Some(tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::TrackNumber) => {
                    if meta.track_number.is_none() {
                        meta.track_number = parse_u32_tag(&tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::DiscNumber) => {
                    if meta.disc_number.is_none() {
                        meta.disc_number = parse_u32_tag(&tag.value.to_string());
                    }
                }
                Some(symphonia::core::meta::StandardTagKey::Date) => {
                    if meta.year.is_none() {
                        meta.year = parse_i32_tag(&tag.value.to_string());
                    }
                }
                _ => {}
            }
        }
        if meta.cover_art.is_none() {
            meta.cover_art = select_cover_art(rev);
        }
    }

    if meta.compilation {
        meta.album_artist = Some("Various Artists".to_string());
    } else if meta.album_artist.is_none() {
        meta.album_artist = meta.artist.clone();
    }

    meta
}

pub fn probe_track(path: &Path) -> Result<TrackMeta> {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !is_supported_extension(&ext) {
        return Err(anyhow::anyhow!("unsupported extension"));
    }
    Ok(probe_track_meta(path, &ext))
}

fn parse_u32_tag(raw: &str) -> Option<u32> {
    raw.split('/')
        .next()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn parse_i32_tag(raw: &str) -> Option<i32> {
    raw.split('-')
        .next()
        .and_then(|s| s.trim().parse::<i32>().ok())
}

fn parse_bool_tag(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y"
    )
}

fn select_cover_art(rev: &symphonia::core::meta::MetadataRevision) -> Option<CoverArt> {
    let mut best = rev
        .visuals()
        .iter()
        .find(|visual| visual.usage == Some(StandardVisualKey::FrontCover));
    if best.is_none() {
        best = rev.visuals().first();
    }
    let visual = best?;
    if visual.data.len() > MAX_COVER_ART_BYTES {
        return None;
    }
    Some(CoverArt {
        mime_type: visual.media_type.clone(),
        data: visual.data.to_vec(),
    })
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn probe_track_meta_sets_format_from_ext() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-library-meta-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let track = root.join("song.flac");
        let _ = std::fs::write(&track, b"test");

        let meta = probe_track_meta(&track, "flac");
        assert_eq!(meta.format, Some("FLAC".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_supported_extension_accepts_known() {
        assert!(is_supported_extension("flac"));
        assert!(is_supported_extension("mp3"));
        assert!(is_supported_extension("opus"));
        assert!(!is_supported_extension("txt"));
    }

    #[test]
    fn scan_library_lists_dirs_and_tracks() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-library-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = root.join("Artists");
        let _ = std::fs::create_dir_all(&dir);
        let track = root.join("song.flac");
        let _ = std::fs::write(&track, b"test");

        let index = scan_library(&root).expect("scan library");
        let entries = index.list_dir(index.root()).expect("entries");
        let names = entries
            .iter()
            .map(|entry| match entry {
                LibraryEntry::Dir { name, .. } => name.clone(),
                LibraryEntry::Track { file_name, .. } => file_name.clone(),
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"Artists".to_string()));
        assert!(names.contains(&"song.flac".to_string()));
    }

    #[test]
    fn find_track_by_path_locates_track() {
        let root = std::env::temp_dir().join(format!(
            "audio-hub-library-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&root);
        let track = root.join("song.flac");
        let _ = std::fs::write(&track, b"test");

        let index = scan_library(&root).expect("scan library");
        let found = index.find_track_by_path(&track.canonicalize().unwrap());
        assert!(matches!(found, Some(LibraryEntry::Track { .. })));
    }
}
