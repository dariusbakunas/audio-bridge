//! Cover art extraction + caching helpers.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::events::{EventBus, MetadataEvent};
use crate::state::MetadataWake;
use crate::library::{CoverArt, TrackMeta};
use crate::metadata_db::{CoverArtCandidate, MetadataDb, TrackRecord};

const COVER_CACHE_DIR: &str = ".audio-hub/art";
const COVER_FILENAMES: [&str; 10] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
    "album.jpg",
];
const CAA_BASE_URL: &str = "https://coverartarchive.org/release";
const CAA_RATE_LIMIT_MS: u64 = 1000;
const MAX_COVER_BYTES: usize = 5_000_000;

#[derive(Clone)]
pub struct CoverArtResolver {
    db: MetadataDb,
    store: CoverArtStore,
    source: CoverArtSource,
}

impl CoverArtResolver {
    pub fn new(db: MetadataDb, root: PathBuf) -> Self {
        Self {
            db,
            store: CoverArtStore::new(root),
            source: CoverArtSource::default(),
        }
    }

    pub fn apply_for_track(
        &self,
        track_path: &Path,
        meta: &TrackMeta,
        record: &TrackRecord,
    ) -> Result<()> {
        let Some(album) = record.album.as_deref() else {
            return Ok(());
        };
        let artist = record.album_artist.as_deref().or(record.artist.as_deref());
        if self.db.album_cover_path(album, artist)?.is_some() {
            return Ok(());
        }

        let cover = self.source.cover_for_track(track_path, meta)?;
        let Some(cover) = cover else {
            return Ok(());
        };

        let hint = match artist {
            Some(artist) => format!("{}-{}", artist, album),
            None => album.to_string(),
        };
        if let Some(existing) = self.store.find_cached_cover(&hint) {
            let _ = self.db.set_album_cover_if_empty(album, artist, &existing)?;
            return Ok(());
        }
        let relative_path = self
            .store
            .store_cover_art(&hint, &cover.mime_type, &cover.data)?;
        let _ = self.db.set_album_cover_if_empty(album, artist, &relative_path)?;
        Ok(())
    }
}

fn read_folder_cover(dir: Option<&Path>) -> Result<Option<CoverArt>> {
    let Some(dir) = dir else {
        return Ok(None);
    };
    for name in COVER_FILENAMES {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let mime_type = match mime_for_extension(path.extension()) {
            Some(mime) => mime.to_string(),
            None => continue,
        };
        let data = std::fs::read(&path)
            .with_context(|| format!("read cover art {:?}", path))?;
        return Ok(Some(CoverArt { mime_type, data }));
    }
    Ok(None)
}

#[derive(Clone)]
struct CoverArtSource {
    strategy: CoverArtStrategy,
}

impl CoverArtSource {
    fn new(strategy: CoverArtStrategy) -> Self {
        Self { strategy }
    }

    fn cover_for_track(&self, track_path: &Path, meta: &TrackMeta) -> Result<Option<CoverArt>> {
        match self.strategy {
            CoverArtStrategy::EmbeddedThenFolder => {
                if let Some(cover) = meta.cover_art.as_ref() {
                    return Ok(Some(cover.clone()));
                }
                read_folder_cover(track_path.parent())
            }
            CoverArtStrategy::FolderThenEmbedded => {
                if let Some(cover) = read_folder_cover(track_path.parent())? {
                    return Ok(Some(cover));
                }
                Ok(meta.cover_art.clone())
            }
            CoverArtStrategy::EmbeddedOnly => Ok(meta.cover_art.clone()),
            CoverArtStrategy::FolderOnly => read_folder_cover(track_path.parent()),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CoverArtStrategy {
    EmbeddedThenFolder,
    FolderThenEmbedded,
    EmbeddedOnly,
    FolderOnly,
}

impl Default for CoverArtSource {
    fn default() -> Self {
        Self::new(CoverArtStrategy::EmbeddedThenFolder)
    }
}

#[derive(Clone)]
struct CoverArtStore {
    root: PathBuf,
}

impl CoverArtStore {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join(COVER_CACHE_DIR)
    }

    fn find_cached_cover(&self, hint: &str) -> Option<String> {
        let slug = slugify(hint);
        let art_dir = self.cache_dir();
        let entries = fs::read_dir(&art_dir).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{slug}-")) {
                let relative = PathBuf::from(COVER_CACHE_DIR).join(name);
                return Some(relative.to_string_lossy().to_string());
            }
        }
        None
    }

    fn store_cover_art(&self, hint: &str, mime_type: &str, data: &[u8]) -> Result<String> {
        let art_dir = self.cache_dir();
        std::fs::create_dir_all(&art_dir)
            .with_context(|| format!("create cover cache {:?}", art_dir))?;

        let ext = extension_for_mime(mime_type).unwrap_or("bin");
        let slug = slugify(hint);
        let hash = hash_bytes(data);
        let filename = format!("{}-{:016x}.{}", slug, hash, ext);
        let relative = PathBuf::from(COVER_CACHE_DIR).join(&filename);
        let full = self.root.join(&relative);
        if !full.exists() {
            std::fs::write(&full, data)
                .with_context(|| format!("write cover art {:?}", full))?;
        }
        Ok(relative.to_string_lossy().to_string())
    }
}

fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let lower = mime.to_ascii_lowercase();
    if lower.contains("jpeg") || lower.contains("jpg") {
        Some("jpg")
    } else if lower.contains("png") {
        Some("png")
    } else if lower.contains("webp") {
        Some("webp")
    } else {
        None
    }
}

fn mime_for_extension(ext: Option<&std::ffi::OsStr>) -> Option<&'static str> {
    let ext = ext?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "cover".to_string()
    } else {
        out
    }
}

fn hash_bytes(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::TrackMeta;
    use crate::metadata_service::MetadataService;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "audio-hub-cover-art-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn slugify_strips_and_collapses() {
        assert_eq!(slugify("A-Ha: Hunting High & Low"), "a-ha-hunting-high-low");
        assert_eq!(slugify("   "), "cover");
        assert_eq!(slugify("--Already--Slug--"), "already-slug");
    }

    #[test]
    fn mime_and_extension_round_trip() {
        assert_eq!(mime_for_extension(Some(std::ffi::OsStr::new("jpg"))), Some("image/jpeg"));
        assert_eq!(mime_for_extension(Some(std::ffi::OsStr::new("png"))), Some("image/png"));
        assert_eq!(extension_for_mime("image/jpeg"), Some("jpg"));
        assert_eq!(extension_for_mime("image/png"), Some("png"));
        assert_eq!(extension_for_mime("application/octet-stream"), None);
    }

    #[test]
    fn cover_store_writes_and_finds_cached_art() {
        let root = temp_root();
        let store = CoverArtStore::new(root.clone());
        let data = b"cover-bytes";
        let hint = "Test Album";
        let relative = store
            .store_cover_art(hint, "image/jpeg", data)
            .expect("store cover art");
        let full = root.join(&relative);
        assert!(full.exists());

        let found = store.find_cached_cover(hint).expect("find cached cover");
        assert_eq!(found, relative);
    }

    #[test]
    fn resolver_skips_when_cover_already_present() {
        let root = temp_root();
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"audio").expect("write file");
        let db = MetadataDb::new(&root).expect("metadata db");
        let fs_meta = std::fs::metadata(&track_path).expect("metadata");
        let meta = TrackMeta {
            album: Some("Album".to_string()),
            artist: Some("Artist".to_string()),
            album_artist: Some("Artist".to_string()),
            cover_art: Some(CoverArt {
                mime_type: "image/jpeg".to_string(),
                data: b"embedded".to_vec(),
            }),
            ..TrackMeta::default()
        };
        let record =
            MetadataService::build_track_record(&track_path, "track.flac", &meta, &fs_meta, None);
        db.upsert_track(&record).expect("upsert track");
        db.set_album_cover_if_empty("Album", Some("Artist"), "existing.jpg")
            .expect("set cover");

        let resolver = CoverArtResolver::new(db, root.clone());
        resolver
            .apply_for_track(&track_path, &meta, &record)
            .expect("apply cover");

        let art_dir = root.join(COVER_CACHE_DIR);
        assert!(!art_dir.exists());
        let cover = resolver
            .db
            .album_cover_path("Album", Some("Artist"))
            .expect("cover path");
        assert_eq!(cover.as_deref(), Some("existing.jpg"));
    }

    #[test]
    fn cover_source_prefers_embedded_when_configured() {
        let root = temp_root();
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"audio").expect("write file");
        let folder_cover = track_path.parent().unwrap().join("cover.jpg");
        std::fs::write(&folder_cover, b"folder").expect("write cover");

        let meta = TrackMeta {
            cover_art: Some(CoverArt {
                mime_type: "image/jpeg".to_string(),
                data: b"embedded".to_vec(),
            }),
            ..TrackMeta::default()
        };
        let source = CoverArtSource::new(CoverArtStrategy::EmbeddedThenFolder);
        let cover = source.cover_for_track(&track_path, &meta).expect("cover");
        assert_eq!(cover.unwrap().data, b"embedded");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cover_source_prefers_folder_when_configured() {
        let root = temp_root();
        let track_path = root.join("track.flac");
        std::fs::write(&track_path, b"audio").expect("write file");
        let folder_cover = track_path.parent().unwrap().join("cover.jpg");
        std::fs::write(&folder_cover, b"folder").expect("write cover");

        let meta = TrackMeta {
            cover_art: Some(CoverArt {
                mime_type: "image/jpeg".to_string(),
                data: b"embedded".to_vec(),
            }),
            ..TrackMeta::default()
        };
        let source = CoverArtSource::new(CoverArtStrategy::FolderThenEmbedded);
        let cover = source.cover_for_track(&track_path, &meta).expect("cover");
        assert_eq!(cover.unwrap().data, b"folder");
        let _ = std::fs::remove_dir_all(root);
    }
}

pub struct CoverArtFetcher {
    db: MetadataDb,
    store: CoverArtStore,
    user_agent: String,
    events: EventBus,
    wake: MetadataWake,
}

impl CoverArtFetcher {
    pub fn new(
        db: MetadataDb,
        root: PathBuf,
        user_agent: String,
        events: EventBus,
        wake: MetadataWake,
    ) -> Self {
        Self {
            db,
            store: CoverArtStore::new(root),
            user_agent,
            events,
            wake,
        }
    }

    pub fn spawn(self) {
        std::thread::spawn(move || {
            let client = CoverArtClient::new(&self.user_agent);
            let mut wake_seq = 0u64;
            loop {
                match self.db.list_cover_art_candidates(25) {
                    Ok(candidates) => {
                        if !candidates.is_empty() {
                            tracing::info!(
                                count = candidates.len(),
                                "cover art candidates fetched"
                            );
                            self.events.metadata_event(MetadataEvent::CoverArtBatch {
                                count: candidates.len(),
                            });
                        }
                        if candidates.is_empty() {
                            self.wake.wait(&mut wake_seq);
                            continue;
                        }
                        for candidate in candidates {
                            if let Err(err) = fetch_and_store_cover(
                                &self.db,
                                &self.store,
                                &client,
                                &self.events,
                                &candidate,
                            ) {
                                tracing::warn!(
                                    error = %err,
                                    album_id = candidate.album_id,
                                    "cover art fetch failed"
                                );
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "cover art candidate query failed");
                        std::thread::sleep(Duration::from_secs(10));
                    }
                }
            }
        });
    }
}

struct CoverArtClient {
    agent: ureq::Agent,
    last_request: Mutex<Instant>,
}

impl CoverArtClient {
    fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(user_agent)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            last_request: Mutex::new(Instant::now() - Duration::from_millis(CAA_RATE_LIMIT_MS)),
        }
    }

    fn fetch_front(&self, mbid: &str) -> Result<(String, Vec<u8>)> {
        self.wait_rate_limit();
        let url = format!("{}/{}/front-500", CAA_BASE_URL, mbid);
        let resp = self
            .agent
            .get(&url)
            .call()
            .context("cover art request failed")?;
        let mime_type = resp
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let bytes = resp
            .into_body()
            .with_config()
            .limit(MAX_COVER_BYTES as u64)
            .read_to_vec()
            .context("cover art read failed")?;
        Ok((mime_type, bytes))
    }

    fn wait_rate_limit(&self) {
        let mut last = self
            .last_request
            .lock()
            .expect("cover art rate limit lock");
        let elapsed = last.elapsed();
        let limit = Duration::from_millis(CAA_RATE_LIMIT_MS);
        if elapsed < limit {
            std::thread::sleep(limit - elapsed);
        }
        *last = Instant::now();
    }
}

pub fn fetch_cover_front(mbid: &str, user_agent: &str) -> Result<(String, Vec<u8>)> {
    let client = CoverArtClient::new(user_agent);
    client.fetch_front(mbid)
}

fn fetch_and_store_cover(
    db: &MetadataDb,
    store: &CoverArtStore,
    client: &CoverArtClient,
    events: &EventBus,
    candidate: &CoverArtCandidate,
) -> Result<()> {
    events.metadata_event(MetadataEvent::CoverArtFetchStart {
        album_id: candidate.album_id,
        mbid: candidate.mbid.clone(),
    });
    tracing::info!(
        album_id = candidate.album_id,
        mbid = %candidate.mbid,
        "cover art fetch start"
    );
    let (mime_type, data) = match client.fetch_front(&candidate.mbid) {
        Ok(result) => result,
        Err(err) => {
            tracing::info!(
                album_id = candidate.album_id,
                mbid = %candidate.mbid,
                error = %err,
                "cover art fetch failed"
            );
            if let Ok(Some(next)) = db.advance_cover_candidate(candidate.album_id) {
                tracing::debug!(
                    album_id = candidate.album_id,
                    next_mbid = %next,
                    "cover art advancing to next release candidate"
                );
                return Ok(());
            }
            tracing::debug!(
                error = %err,
                album_id = candidate.album_id,
                "cover art fetch returned no image"
            );
            let attempts = db.increment_cover_art_fail(candidate.album_id, &err.to_string())?;
            events.metadata_event(MetadataEvent::CoverArtFetchFailure {
                album_id: candidate.album_id,
                mbid: candidate.mbid.clone(),
                error: err.to_string(),
                attempts,
            });
            return Ok(());
        }
    };
    if data.is_empty() {
        tracing::info!(
            album_id = candidate.album_id,
            mbid = %candidate.mbid,
            "cover art fetch empty response"
        );
        let attempts = db.increment_cover_art_fail(candidate.album_id, "empty response")?;
        events.metadata_event(MetadataEvent::CoverArtFetchFailure {
            album_id: candidate.album_id,
            mbid: candidate.mbid.clone(),
            error: "empty response".to_string(),
            attempts,
        });
        return Ok(());
    }
    let hint = format!("album-{}", candidate.album_id);
    let relative_path = store.store_cover_art(&hint, &mime_type, &data)?;
    let updated = db.set_album_cover_by_id_if_empty(candidate.album_id, &relative_path)?;
    if updated {
        tracing::info!(
            album_id = candidate.album_id,
            cover_path = %relative_path,
            "cover art stored"
        );
        events.metadata_event(MetadataEvent::CoverArtFetchSuccess {
            album_id: candidate.album_id,
            cover_path: relative_path,
        });
        events.library_changed();
    } else {
        tracing::info!(
            album_id = candidate.album_id,
            cover_path = %relative_path,
            "cover art fetched but album already has cover"
        );
    }
    Ok(())
}
