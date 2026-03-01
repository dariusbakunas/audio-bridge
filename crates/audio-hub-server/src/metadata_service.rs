//! Shared metadata operations (scan/rescan/update helpers).

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::HttpResponse;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cover_art::CoverArtResolver;
use crate::events::{EventBus, MetadataEvent};
use crate::library::{LibraryIndex, TrackMeta, probe_track, scan_library_with_meta};
use crate::metadata_db::{AlbumSummary, MetadataDb, TrackRecord};
use crate::state::MetadataWake;

#[derive(Clone)]
/// High-level metadata orchestration service over DB + filesystem.
pub struct MetadataService {
    db: MetadataDb,
    cover_art: CoverArtResolver,
    events: EventBus,
    metadata_wake: MetadataWake,
    root: PathBuf,
}

const ALBUM_MARKER_DIR: &str = ".audio-hub";
const ALBUM_MARKER_FILE: &str = "album.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
/// On-disk album folder marker used to keep stable album UUID grouping.
struct AlbumFolderMarker {
    album_uuid: String,
    title: Option<String>,
    artist: Option<String>,
    original_year: Option<i32>,
    created_at_ms: i64,
}

impl MetadataService {
    /// Create a metadata service bound to a library root and event buses.
    pub fn new(
        db: MetadataDb,
        root: PathBuf,
        events: EventBus,
        metadata_wake: MetadataWake,
    ) -> Self {
        Self {
            db: db.clone(),
            cover_art: CoverArtResolver::new(db, root.clone()),
            events,
            metadata_wake,
            root,
        }
    }

    /// Build a DB track record from probed metadata + filesystem metadata.
    pub fn build_track_record(
        path: &Path,
        file_name: &str,
        meta: &TrackMeta,
        fs_meta: &std::fs::Metadata,
        album_uuid: Option<String>,
    ) -> TrackRecord {
        let (album, disc_number, _source) = normalize_album_and_disc(path, meta);
        TrackRecord {
            path: path.to_string_lossy().to_string(),
            file_name: file_name.to_string(),
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            album_artist: meta.album_artist.clone(),
            album,
            album_uuid,
            track_number: meta.track_number,
            disc_number,
            year: meta.year,
            duration_ms: meta.duration_ms,
            sample_rate: meta.sample_rate,
            bit_depth: meta.bit_depth,
            format: meta.format.clone(),
            mtime_ms: fs_meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            size_bytes: fs_meta.len() as i64,
        }
    }

    /// Upsert one track record and resolve/update its cover art metadata.
    pub fn upsert_track_record(
        &self,
        path: &Path,
        meta: &TrackMeta,
        record: &TrackRecord,
    ) -> Result<(), String> {
        self.db
            .upsert_track(record)
            .map_err(|err| err.to_string())?;
        if let Err(err) = self.cover_art.apply_for_track(path, meta, record) {
            tracing::warn!(error = %err, path = %record.path, "cover art apply failed");
        }
        Ok(())
    }

    /// Rescan a single track file and update DB + in-memory index.
    pub fn rescan_track(
        &self,
        library: &RwLock<LibraryIndex>,
        full_path: &Path,
    ) -> Result<(), HttpResponse> {
        let fs_meta = match std::fs::metadata(full_path) {
            Ok(meta) => meta,
            Err(_) => return Err(HttpResponse::NotFound().finish()),
        };
        let mtime_ms = fs_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let size_bytes = fs_meta.len() as i64;
        if let Ok(Some(existing)) = self.db.track_record_by_path(&full_path.to_string_lossy()) {
            if existing.mtime_ms == mtime_ms && existing.size_bytes == size_bytes {
                if let Ok(index) = library.read() {
                    if index.find_track_by_path(full_path).is_some() {
                        return Ok(());
                    }
                }
            }
        }
        let meta = match probe_track(full_path) {
            Ok(meta) => meta,
            Err(err) => return Err(HttpResponse::BadRequest().body(err.to_string())),
        };
        let mut normalized_meta = meta.clone();
        let original_album = normalized_meta.album.clone();
        let (album, disc_number, source) = normalize_album_and_disc(full_path, &normalized_meta);
        if let (Some(original), Some(normalized), Some(source)) =
            (original_album, album.clone(), source)
        {
            if original != normalized {
                let track_id = self
                    .db
                    .track_id_for_path(&full_path.to_string_lossy())
                    .ok()
                    .flatten();
                self.events
                    .metadata_event(MetadataEvent::AlbumNormalization {
                        track_id,
                        original_album: original,
                        normalized_album: normalized.clone(),
                        disc_number,
                        source: source.to_string(),
                    });
            }
        }
        normalized_meta.album = album;
        normalized_meta.disc_number = disc_number;
        let album_uuid = self.album_uuid_for_track(full_path, &normalized_meta);
        let file_name = full_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>");
        let ext_hint = full_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let record =
            Self::build_track_record(full_path, file_name, &normalized_meta, &fs_meta, album_uuid);
        let _ = self.db.clear_musicbrainz_no_match(&record.path);
        if let Err(err) = self.upsert_track_record(full_path, &normalized_meta, &record) {
            return Err(HttpResponse::InternalServerError().body(err));
        }
        if let Ok(mut index) = library.write() {
            if !index.update_track_meta(full_path, &normalized_meta) {
                index.upsert_track_entry(full_path, file_name, &ext_hint, &normalized_meta);
            }
        }
        self.events.library_changed();
        self.metadata_wake.notify();
        Ok(())
    }

    /// Fetch one track record by path.
    pub fn track_record_by_path(&self, path: &str) -> Result<Option<TrackRecord>, String> {
        self.db
            .track_record_by_path(path)
            .map_err(|err| err.to_string())
    }

    /// Fetch one track record by id.
    pub fn track_record_by_id(&self, track_id: i64) -> Result<Option<TrackRecord>, String> {
        self.db
            .track_record_by_id(track_id)
            .map_err(|err| err.to_string())
    }

    /// Fetch album summary by id.
    pub fn album_summary_by_id(&self, album_id: i64) -> Result<Option<AlbumSummary>, String> {
        self.db
            .album_summary_by_id(album_id)
            .map_err(|err| err.to_string())
    }

    /// List all track paths currently assigned to an album.
    pub fn list_track_paths_by_album_id(&self, album_id: i64) -> Result<Vec<String>, String> {
        self.db
            .list_track_paths_by_album_id(album_id)
            .map_err(|err| err.to_string())
    }

    /// Update album-level metadata fields and return resulting album id.
    pub fn update_album_metadata(
        &self,
        album_id: i64,
        title: Option<&str>,
        artist: Option<&str>,
        year: Option<i32>,
    ) -> Result<Option<i64>, String> {
        self.db
            .update_album_metadata(album_id, title, artist, year)
            .map_err(|err| format!("{err:#}"))
    }

    /// Resolve album id for one track path.
    pub fn album_id_for_track_path(&self, path: &str) -> Result<Option<i64>, String> {
        self.db
            .album_id_for_track_path(path)
            .map_err(|err| err.to_string())
    }

    /// Resolve cover-art path for one track path.
    pub fn cover_path_for_track(&self, path: &str) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_track(path)
            .map_err(|err| err.to_string())
    }

    /// Resolve cover-art path for one track id.
    pub fn cover_path_for_track_id(&self, track_id: i64) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_track_id(track_id)
            .map_err(|err| err.to_string())
    }

    /// Resolve cover-art path for one album id.
    pub fn cover_path_for_album_id(&self, album_id: i64) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_album_id(album_id)
            .map_err(|err| err.to_string())
    }

    /// Full library rescan plus stale-track pruning and marker backfill.
    pub fn rescan_library(&self, emit_events: bool) -> Result<LibraryIndex> {
        let (index, seen_paths) = self.scan_library_with_paths(emit_events)?;
        let existing = self.db.list_all_track_paths()?;
        for path in existing {
            if !seen_paths.contains(path.as_str()) {
                let _ = self.db.delete_track_by_path(&path);
            }
        }
        self.db.prune_orphaned_albums_and_artists()?;
        self.ensure_album_markers();
        Ok(index)
    }

    /// Internal scan returning both index and seen path set.
    fn scan_library_with_paths(
        &self,
        emit_events: bool,
    ) -> Result<(LibraryIndex, std::collections::HashSet<String>)> {
        let mut seen = std::collections::HashSet::new();
        let index = scan_library_with_meta(
            &self.root,
            |path, file_name, _ext, meta, fs_meta| {
                seen.insert(path.to_string_lossy().to_string());
                let mut normalized_meta = meta.clone();
                let original_album = normalized_meta.album.clone();
                let (album, disc_number, source) = normalize_album_and_disc(path, &normalized_meta);
                if let (Some(original), Some(normalized), Some(source)) =
                    (original_album, album.clone(), source)
                {
                    if original != normalized {
                        let track_id = self
                            .db
                            .track_id_for_path(&path.to_string_lossy())
                            .ok()
                            .flatten();
                        self.events
                            .metadata_event(MetadataEvent::AlbumNormalization {
                                track_id,
                                original_album: original,
                                normalized_album: normalized.clone(),
                                disc_number,
                                source: source.to_string(),
                            });
                    }
                }
                normalized_meta.album = album;
                normalized_meta.disc_number = disc_number;
                let album_uuid = self.album_uuid_for_track(path, &normalized_meta);
                let record = Self::build_track_record(
                    path,
                    file_name,
                    &normalized_meta,
                    fs_meta,
                    album_uuid,
                );
                if let Err(err) = self.upsert_track_record(path, &normalized_meta, &record) {
                    tracing::warn!(error = %err, path = %record.path, "metadata upsert failed");
                }
            },
            |dir, count| {
                if !emit_events {
                    return;
                }
                let album = dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| dir.to_string_lossy().to_string());
                if count == 0 {
                    self.events
                        .metadata_event(MetadataEvent::LibraryScanAlbumStart { album });
                } else {
                    self.events
                        .metadata_event(MetadataEvent::LibraryScanAlbumFinish {
                            album,
                            tracks: count,
                        });
                }
            },
        )?;
        Ok((index, seen))
    }

    /// Remove one track path from DB and in-memory index.
    pub fn remove_track_by_path(
        &self,
        library: &RwLock<LibraryIndex>,
        raw_path: &Path,
    ) -> Result<bool, HttpResponse> {
        let mut normalized = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else {
            self.root.join(raw_path)
        };
        if let Ok(path) = normalized.canonicalize() {
            normalized = path;
        }
        if !normalized.starts_with(&self.root) {
            return Err(HttpResponse::BadRequest().body("path outside library root"));
        }
        let path_str = normalized.to_string_lossy().to_string();
        let deleted = self
            .db
            .delete_track_by_path(&path_str)
            .map_err(|err| HttpResponse::InternalServerError().body(err.to_string()))?;
        if deleted {
            if let Ok(mut index) = library.write() {
                index.remove_track(&normalized);
            }
            if let Err(err) = self.db.prune_orphaned_albums_and_artists() {
                tracing::warn!(error = %err, "metadata prune failed");
            }
            self.events.library_changed();
            self.metadata_wake.notify();
        }
        Ok(deleted)
    }

    /// Scan library and return fresh index (without stale-track pruning).
    pub fn scan_library(&self, emit_events: bool) -> Result<LibraryIndex> {
        let (index, _) = self.scan_library_with_paths(emit_events)?;
        Ok(index)
    }

    /// Create album marker files for albums that have UUIDs but missing markers.
    pub fn ensure_album_markers(&self) {
        let candidates = match self.db.album_marker_candidates() {
            Ok(list) => list,
            Err(err) => {
                tracing::warn!(error = %err, "album marker candidate query failed");
                return;
            }
        };
        for candidate in candidates {
            let path = PathBuf::from(candidate.path);
            let Some(dir) = path.parent() else { continue };
            if read_album_marker(dir).is_some() {
                continue;
            }
            let marker = AlbumFolderMarker {
                album_uuid: candidate.album_uuid,
                title: candidate.title,
                artist: candidate.artist,
                original_year: candidate.original_year.or(candidate.year),
                created_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            };
            if let Err(err) = write_album_marker(dir, &marker) {
                tracing::warn!(error = %err, path = %dir.display(), "album marker write failed");
            }
        }
    }

    /// Resolve and validate a track path under the library root.
    pub fn resolve_track_path(root: &Path, raw_path: &str) -> Result<PathBuf, HttpResponse> {
        let raw_path = PathBuf::from(raw_path);
        let candidate = if raw_path.is_absolute() {
            raw_path
        } else {
            root.join(raw_path)
        };
        let full_path = match candidate.canonicalize() {
            Ok(path) => path,
            Err(_) => return Err(HttpResponse::NotFound().finish()),
        };
        if !full_path.starts_with(root) {
            return Err(HttpResponse::BadRequest().body("path outside library root"));
        }
        if !full_path.is_file() {
            return Err(HttpResponse::NotFound().finish());
        }
        Ok(full_path)
    }

    /// Resolve stable album UUID for a track using marker, DB, or generated UUID.
    fn album_uuid_for_track(&self, path: &Path, meta: &TrackMeta) -> Option<String> {
        let Some(album_title) = meta
            .album
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            return None;
        };
        let dir = path.parent()?;
        if let Some(marker) = read_album_marker(dir) {
            return Some(marker.album_uuid);
        }
        let album_artist = meta.album_artist.as_deref().or(meta.artist.as_deref());
        let existing = self
            .db
            .album_uuid_for_title_artist(album_title, album_artist)
            .ok()
            .flatten();
        let album_uuid = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
        let marker = AlbumFolderMarker {
            album_uuid: album_uuid.clone(),
            title: Some(album_title.to_string()),
            artist: album_artist.map(|value| value.to_string()),
            original_year: meta.year,
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        };
        if let Err(err) = write_album_marker(dir, &marker) {
            tracing::warn!(error = %err, path = %dir.display(), "album marker write failed");
        }
        Some(album_uuid)
    }
}

/// Normalize album names containing disc suffixes and infer disc number.
fn normalize_album_and_disc(
    path: &Path,
    meta: &TrackMeta,
) -> (Option<String>, Option<u32>, Option<&'static str>) {
    let mut album = meta.album.clone();
    let mut disc_number = meta.disc_number;
    let Some(raw_album) = meta.album.as_deref() else {
        return (album, disc_number, None);
    };
    let Some((normalized, disc_from_suffix)) = extract_disc_suffix(raw_album) else {
        return (album, disc_number, None);
    };

    let path_hint = disc_hint_from_path(path);
    if disc_number.is_none() {
        if path_hint != Some(disc_from_suffix) {
            return (album, disc_number, None);
        }
        disc_number = Some(disc_from_suffix);
    }

    if disc_number == Some(disc_from_suffix) {
        album = Some(normalized);
    }

    let source = if meta.disc_number.is_some() {
        Some("tag")
    } else if path_hint.is_some() {
        Some("folder")
    } else {
        None
    };

    (album, disc_number, source)
}

/// Compute marker file path under an album directory.
fn album_marker_path(dir: &Path) -> PathBuf {
    dir.join(ALBUM_MARKER_DIR).join(ALBUM_MARKER_FILE)
}

/// Read album marker file when present and valid.
fn read_album_marker(dir: &Path) -> Option<AlbumFolderMarker> {
    let path = album_marker_path(dir);
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Write album marker file once (no-op if marker already exists).
fn write_album_marker(dir: &Path, marker: &AlbumFolderMarker) -> Result<(), String> {
    let marker_dir = dir.join(ALBUM_MARKER_DIR);
    std::fs::create_dir_all(&marker_dir).map_err(|err| err.to_string())?;
    let path = marker_dir.join(ALBUM_MARKER_FILE);
    if path.exists() {
        return Ok(());
    }
    let data = serde_json::to_string_pretty(marker).map_err(|err| err.to_string())?;
    std::fs::write(path, data).map_err(|err| err.to_string())
}

/// Extract `(normalized_album_title, disc_number)` from common suffix formats.
fn extract_disc_suffix(raw: &str) -> Option<(String, u32)> {
    let trimmed = raw.trim();
    if let Some((left, right)) = split_suffix_with_delim(trimmed, " - ") {
        if let Some(disc) = parse_disc_number(right) {
            return Some((left.to_string(), disc));
        }
    }

    if let Some((left, suffix)) = split_wrapped_suffix(trimmed) {
        if let Some(disc) = parse_disc_number(suffix) {
            return Some((left.to_string(), disc));
        }
    }

    if let Some((left, suffix)) = split_suffix_with_space(trimmed) {
        if let Some(disc) = parse_disc_number(suffix) {
            return Some((left.to_string(), disc));
        }
    }

    None
}

/// Split trailing wrapped suffix like `Album (Disc 2)` or `Album [CD1]`.
fn split_wrapped_suffix(raw: &str) -> Option<(&str, &str)> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (open, _close) = match bytes[bytes.len() - 1] {
        b')' => (b'(', b')'),
        b']' => (b'[', b']'),
        b'}' => (b'{', b'}'),
        _ => return None,
    };
    let mut idx = None;
    for (i, ch) in bytes.iter().enumerate() {
        if *ch == open {
            idx = Some(i);
        }
    }
    let start = idx?;
    let left = raw[..start].trim_end();
    let suffix = raw[start + 1..raw.len() - 1].trim();
    if left.is_empty() || suffix.is_empty() {
        None
    } else {
        Some((left, suffix))
    }
}

/// Split trailing suffix by explicit delimiter.
fn split_suffix_with_delim<'a>(raw: &'a str, delim: &'a str) -> Option<(&'a str, &'a str)> {
    let (left, right) = raw.rsplit_once(delim)?;
    let left = left.trim_end();
    let right = right.trim_start();
    if left.is_empty() || right.is_empty() {
        None
    } else {
        Some((left, right))
    }
}

/// Split trailing suffix by final whitespace token.
fn split_suffix_with_space(raw: &str) -> Option<(&str, &str)> {
    let (left, right) = raw.rsplit_once(' ')?;
    let left = left.trim_end();
    let right = right.trim_start();
    if left.is_empty() || right.is_empty() {
        None
    } else {
        Some((left, right))
    }
}

/// Parse disc number from free-form tokens (`disc 2`, `cd1`, etc.).
fn parse_disc_number(raw: &str) -> Option<u32> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if let Ok(value) = lower.parse::<u32>() {
        return Some(value);
    }
    for key in ["disc", "disk", "cd"] {
        if let Some(rest) = lower.strip_prefix(key) {
            let rest = rest.trim_start_matches(|c: char| !c.is_ascii_digit());
            if let Some(value) = rest.split_whitespace().next() {
                if let Ok(num) = value.parse::<u32>() {
                    return Some(num);
                }
            }
        }
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    for window in tokens.windows(2) {
        if ["disc", "disk", "cd"].contains(&window[0]) {
            if let Ok(num) = window[1].parse::<u32>() {
                return Some(num);
            }
        }
    }
    None
}

/// Infer disc number from parent folder naming conventions.
fn disc_hint_from_path(path: &Path) -> Option<u32> {
    let name = path.parent()?.file_name()?.to_string_lossy().to_string();
    parse_disc_number(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "audio-hub-metadata-service-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn build_track_record_copies_meta_and_fs_fields() {
        let root = temp_root();
        let path = root.join("song.flac");
        std::fs::write(&path, b"audio").expect("write file");
        let fs_meta = std::fs::metadata(&path).expect("metadata");

        let meta = TrackMeta {
            title: Some("Title".to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Album Artist".to_string()),
            track_number: Some(3),
            disc_number: Some(1),
            year: Some(1999),
            duration_ms: Some(1234),
            sample_rate: Some(44100),
            bit_depth: Some(24),
            format: Some("FLAC".to_string()),
            ..TrackMeta::default()
        };

        let record = MetadataService::build_track_record(&path, "song.flac", &meta, &fs_meta, None);
        assert_eq!(record.title.as_deref(), Some("Title"));
        assert_eq!(record.artist.as_deref(), Some("Artist"));
        assert_eq!(record.album.as_deref(), Some("Album"));
        assert_eq!(record.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(record.track_number, Some(3));
        assert_eq!(record.disc_number, Some(1));
        assert_eq!(record.year, Some(1999));
        assert_eq!(record.duration_ms, Some(1234));
        assert_eq!(record.sample_rate, Some(44100));
        assert_eq!(record.bit_depth, Some(24));
        assert_eq!(record.format.as_deref(), Some("FLAC"));
        assert_eq!(record.size_bytes, fs_meta.len() as i64);
    }

    #[test]
    fn resolve_track_path_rejects_outside_root() {
        let root = temp_root().canonicalize().expect("canonicalize root");
        let other = temp_root().join("outside.flac");
        std::fs::write(&other, b"audio").expect("write file");

        let result = MetadataService::resolve_track_path(&root, &other.to_string_lossy());
        assert!(
            matches!(result, Err(resp) if resp.status() == actix_web::http::StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn resolve_track_path_accepts_file_under_root() {
        let root = temp_root().canonicalize().expect("canonicalize root");
        let path = root.join("inside.flac");
        std::fs::write(&path, b"audio").expect("write file");

        let result = MetadataService::resolve_track_path(&root, &path.to_string_lossy());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), path.canonicalize().unwrap());
    }

    #[test]
    fn resolve_track_path_accepts_relative_path_under_root() {
        let root = temp_root().canonicalize().expect("canonicalize root");
        let nested = root.join("album").join("inside.flac");
        std::fs::create_dir_all(nested.parent().expect("parent")).expect("create album dir");
        std::fs::write(&nested, b"audio").expect("write file");

        let result = MetadataService::resolve_track_path(&root, "album/inside.flac");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), nested.canonicalize().unwrap());
    }

    #[test]
    fn normalize_album_disc_suffix_sets_disc_from_folder_hint() {
        let root = temp_root();
        let cd_dir = root.join("CD1");
        std::fs::create_dir_all(&cd_dir).expect("create cd dir");
        let path = cd_dir.join("track.flac");

        let meta = TrackMeta {
            album: Some("Random Access Memories (1)".to_string()),
            ..TrackMeta::default()
        };

        let (album, disc, _source) = normalize_album_and_disc(&path, &meta);
        assert_eq!(album.as_deref(), Some("Random Access Memories"));
        assert_eq!(disc, Some(1));
    }

    #[test]
    fn normalize_album_disc_suffix_skips_when_no_hint_and_no_disc() {
        let root = temp_root();
        let path = root.join("track.flac");
        let meta = TrackMeta {
            album: Some("Random Access Memories (1)".to_string()),
            ..TrackMeta::default()
        };

        let (album, disc, _source) = normalize_album_and_disc(&path, &meta);
        assert_eq!(album.as_deref(), Some("Random Access Memories (1)"));
        assert_eq!(disc, None);
    }

    #[test]
    fn normalize_album_disc_suffix_respects_existing_disc_number() {
        let root = temp_root();
        let path = root.join("track.flac");
        let meta = TrackMeta {
            album: Some("Random Access Memories (2)".to_string()),
            disc_number: Some(2),
            ..TrackMeta::default()
        };

        let (album, disc, _source) = normalize_album_and_disc(&path, &meta);
        assert_eq!(album.as_deref(), Some("Random Access Memories"));
        assert_eq!(disc, Some(2));
    }
}
