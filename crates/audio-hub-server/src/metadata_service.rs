//! Shared metadata operations (scan/rescan/update helpers).

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use actix_web::HttpResponse;
use anyhow::Result;

use crate::cover_art::CoverArtResolver;
use crate::events::{EventBus, MetadataEvent};
use crate::library::{probe_track, scan_library_with_meta, LibraryIndex, TrackMeta};
use crate::metadata_db::{AlbumSummary, MetadataDb, TrackRecord};
use crate::state::MetadataWake;

#[derive(Clone)]
pub struct MetadataService {
    db: MetadataDb,
    cover_art: CoverArtResolver,
    events: EventBus,
    metadata_wake: MetadataWake,
    root: PathBuf,
}

impl MetadataService {
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

    pub fn build_track_record(
        path: &Path,
        file_name: &str,
        meta: &TrackMeta,
        fs_meta: &std::fs::Metadata,
    ) -> TrackRecord {
        TrackRecord {
            path: path.to_string_lossy().to_string(),
            file_name: file_name.to_string(),
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            album_artist: meta.album_artist.clone(),
            album: meta.album.clone(),
            track_number: meta.track_number,
            disc_number: meta.disc_number,
            year: meta.year,
            duration_ms: meta.duration_ms,
            sample_rate: meta.sample_rate,
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
        let file_name = full_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>");
        let record = Self::build_track_record(full_path, file_name, &meta, &fs_meta);
        let _ = self.db.clear_musicbrainz_no_match(&record.path);
        if let Err(err) = self.upsert_track_record(full_path, &meta, &record) {
            return Err(HttpResponse::InternalServerError().body(err));
        }
        if let Ok(mut index) = library.write() {
            index.update_track_meta(full_path, &meta);
        }
        self.events.library_changed();
        self.metadata_wake.notify();
        Ok(())
    }

    pub fn clear_library(&self) -> Result<()> {
        self.db.clear_library()
    }

    pub fn track_record_by_path(&self, path: &str) -> Result<Option<TrackRecord>, String> {
        self.db
            .track_record_by_path(path)
            .map_err(|err| err.to_string())
    }

    pub fn album_summary_by_id(&self, album_id: i64) -> Result<Option<AlbumSummary>, String> {
        self.db
            .album_summary_by_id(album_id)
            .map_err(|err| err.to_string())
    }

    pub fn list_track_paths_by_album_id(&self, album_id: i64) -> Result<Vec<String>, String> {
        self.db
            .list_track_paths_by_album_id(album_id)
            .map_err(|err| err.to_string())
    }

    pub fn update_album_metadata(
        &self,
        album_id: i64,
        title: Option<&str>,
        artist: Option<&str>,
        year: Option<i32>,
    ) -> Result<bool, String> {
        self.db
            .update_album_metadata(album_id, title, artist, year)
            .map_err(|err| err.to_string())
    }

    pub fn album_id_for_track_path(&self, path: &str) -> Result<Option<i64>, String> {
        self.db
            .album_id_for_track_path(path)
            .map_err(|err| err.to_string())
    }

    pub fn cover_path_for_track(&self, path: &str) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_track(path)
            .map_err(|err| err.to_string())
    }

    pub fn cover_path_for_track_id(&self, track_id: i64) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_track_id(track_id)
            .map_err(|err| err.to_string())
    }

    pub fn cover_path_for_album_id(&self, album_id: i64) -> Result<Option<String>, String> {
        self.db
            .cover_path_for_album_id(album_id)
            .map_err(|err| err.to_string())
    }

    pub fn rescan_library(&self, emit_events: bool) -> Result<LibraryIndex> {
        let (index, seen_paths) = self.scan_library_with_paths(emit_events)?;
        let existing = self.db.list_all_track_paths()?;
        for path in existing {
            if !seen_paths.contains(path.as_str()) {
                let _ = self.db.delete_track_by_path(&path);
            }
        }
        self.db.prune_orphaned_albums_and_artists()?;
        Ok(index)
    }

    fn scan_library_with_paths(
        &self,
        emit_events: bool,
    ) -> Result<(LibraryIndex, std::collections::HashSet<String>)> {
        let mut seen = std::collections::HashSet::new();
        let index = scan_library_with_meta(
            &self.root,
            |path, file_name, _ext, meta, fs_meta| {
                seen.insert(path.to_string_lossy().to_string());
                let record = Self::build_track_record(path, file_name, meta, fs_meta);
                if let Err(err) = self.upsert_track_record(path, meta, &record) {
                    tracing::warn!(error = %err, path = %record.path, "metadata upsert failed");
                }
            },
            |dir, count| {
                if !emit_events {
                    return;
                }
                let path = dir.to_string_lossy().to_string();
                if count == 0 {
                    self.events.metadata_event(MetadataEvent::LibraryScanAlbumStart { path });
                } else {
                    self.events.metadata_event(MetadataEvent::LibraryScanAlbumFinish {
                        path,
                        tracks: count,
                    });
                }
            },
        )?;
        Ok((index, seen))
    }

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

    pub fn scan_library(&self, emit_events: bool) -> Result<LibraryIndex> {
        let (index, _) = self.scan_library_with_paths(emit_events)?;
        Ok(index)
    }

    pub fn resolve_track_path(root: &Path, raw_path: &str) -> Result<PathBuf, HttpResponse> {
        let raw_path = PathBuf::from(raw_path);
        let full_path = match raw_path.canonicalize() {
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
            format: Some("FLAC".to_string()),
            ..TrackMeta::default()
        };

        let record = MetadataService::build_track_record(&path, "song.flac", &meta, &fs_meta);
        assert_eq!(record.title.as_deref(), Some("Title"));
        assert_eq!(record.artist.as_deref(), Some("Artist"));
        assert_eq!(record.album.as_deref(), Some("Album"));
        assert_eq!(record.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(record.track_number, Some(3));
        assert_eq!(record.disc_number, Some(1));
        assert_eq!(record.year, Some(1999));
        assert_eq!(record.duration_ms, Some(1234));
        assert_eq!(record.sample_rate, Some(44100));
        assert_eq!(record.format.as_deref(), Some("FLAC"));
        assert_eq!(record.size_bytes, fs_meta.len() as i64);
    }

    #[test]
    fn resolve_track_path_rejects_outside_root() {
        let root = temp_root().canonicalize().expect("canonicalize root");
        let other = temp_root().join("outside.flac");
        std::fs::write(&other, b"audio").expect("write file");

        let result = MetadataService::resolve_track_path(&root, &other.to_string_lossy());
        assert!(matches!(result, Err(resp) if resp.status() == actix_web::http::StatusCode::BAD_REQUEST));
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
}
