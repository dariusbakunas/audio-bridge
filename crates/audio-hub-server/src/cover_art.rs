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

pub fn apply_cover_art(
    db: &MetadataDb,
    root: &Path,
    track_path: &Path,
    meta: &TrackMeta,
    record: &TrackRecord,
) -> Result<()> {
    let Some(album) = record.album.as_deref() else {
        return Ok(());
    };
    let artist = record.album_artist.as_deref().or(record.artist.as_deref());
    if db.album_cover_path(album, artist)?.is_some() {
        return Ok(());
    }

    let cover = if let Some(cover) = meta.cover_art.as_ref() {
        Some(cover.clone())
    } else {
        read_folder_cover(track_path.parent())?
    };
    let Some(cover) = cover else {
        return Ok(());
    };

    let hint = match artist {
        Some(artist) => format!("{}-{}", artist, album),
        None => album.to_string(),
    };
    if let Some(existing) = find_cached_cover(root, &hint) {
        let _ = db.set_album_cover_if_empty(album, artist, &existing)?;
        return Ok(());
    }
    let relative_path = store_cover_art(root, &hint, &cover.mime_type, &cover.data)?;
    let _ = db.set_album_cover_if_empty(album, artist, &relative_path)?;
    Ok(())
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

fn find_cached_cover(root: &Path, hint: &str) -> Option<String> {
    let slug = slugify(hint);
    let art_dir = root.join(COVER_CACHE_DIR);
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

pub fn store_cover_art(
    root: &Path,
    hint: &str,
    mime_type: &str,
    data: &[u8],
) -> Result<String> {
    let art_dir = root.join(COVER_CACHE_DIR);
    std::fs::create_dir_all(&art_dir)
        .with_context(|| format!("create cover cache {:?}", art_dir))?;

    let ext = extension_for_mime(mime_type).unwrap_or("bin");
    let slug = slugify(hint);
    let hash = hash_bytes(data);
    let filename = format!("{}-{:016x}.{}", slug, hash, ext);
    let relative = PathBuf::from(COVER_CACHE_DIR).join(&filename);
    let full = root.join(&relative);
    if !full.exists() {
        std::fs::write(&full, data)
            .with_context(|| format!("write cover art {:?}", full))?;
    }
    Ok(relative.to_string_lossy().to_string())
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

pub fn spawn_caa_loop(
    db: MetadataDb,
    root: PathBuf,
    user_agent: String,
    events: EventBus,
    wake: MetadataWake,
) {
    std::thread::spawn(move || {
        let client = CoverArtClient::new(&user_agent);
        let mut wake_seq = 0u64;
        loop {
            match db.list_cover_art_candidates(25) {
                Ok(candidates) => {
                    if !candidates.is_empty() {
                        tracing::info!(
                            count = candidates.len(),
                            "cover art candidates fetched"
                        );
                        events.metadata_event(MetadataEvent::CoverArtBatch {
                            count: candidates.len(),
                        });
                    }
                    if candidates.is_empty() {
                        wake.wait(&mut wake_seq);
                        continue;
                    }
                    for candidate in candidates {
                        if let Err(err) = fetch_and_store_cover(
                            &db,
                            &root,
                            &client,
                            &events,
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

fn fetch_and_store_cover(
    db: &MetadataDb,
    root: &Path,
    client: &CoverArtClient,
    events: &EventBus,
    candidate: &CoverArtCandidate,
) -> Result<()> {
    events.metadata_event(MetadataEvent::CoverArtFetchStart {
        album_id: candidate.album_id,
        mbid: candidate.mbid.clone(),
    });
    let (mime_type, data) = match client.fetch_front(&candidate.mbid) {
        Ok(result) => result,
        Err(err) => {
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
    let relative_path = store_cover_art(root, &hint, &mime_type, &data)?;
    let updated = db.set_album_cover_by_id_if_empty(candidate.album_id, &relative_path)?;
    if updated {
        events.metadata_event(MetadataEvent::CoverArtFetchSuccess {
            album_id: candidate.album_id,
            cover_path: relative_path,
        });
        events.library_changed();
    }
    Ok(())
}
