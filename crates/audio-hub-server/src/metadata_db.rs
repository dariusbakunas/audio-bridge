//! SQLite metadata store for artists/albums/tracks.
//!
//! Provides pooled connections and schema bootstrap.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, params};

use crate::musicbrainz::MusicBrainzMatch;
use uuid::Uuid;
const SCHEMA_VERSION: i32 = 10;

#[derive(Clone)]
/// SQLite-backed metadata database handle with pooled connections.
pub struct MetadataDb {
    pool: Pool<SqliteConnectionManager>,
    media_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
/// Internal track record used for upsert/update operations.
pub struct TrackRecord {
    /// Absolute or media-root-relative file path.
    pub path: String,
    /// Filename for display.
    pub file_name: String,
    /// Track title.
    pub title: Option<String>,
    /// Track artist.
    pub artist: Option<String>,
    /// Album artist.
    pub album_artist: Option<String>,
    /// Album title.
    pub album: Option<String>,
    /// Stable album UUID for grouping.
    pub album_uuid: Option<String>,
    /// Track number.
    pub track_number: Option<u32>,
    /// Disc number.
    pub disc_number: Option<u32>,
    /// Release year.
    pub year: Option<i32>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Bit depth.
    pub bit_depth: Option<u32>,
    /// Format label.
    pub format: Option<String>,
    /// File mtime (unix ms).
    pub mtime_ms: i64,
    /// File size in bytes.
    pub size_bytes: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
/// Artist summary row returned by list endpoints.
pub struct ArtistSummary {
    /// Artist id.
    pub id: i64,
    /// Stable artist UUID.
    pub uuid: Option<String>,
    /// Display artist name.
    pub name: String,
    /// Optional sort name.
    pub sort_name: Option<String>,
    /// Optional MusicBrainz artist MBID.
    pub mbid: Option<String>,
    /// Album count for this artist.
    pub album_count: i64,
    /// Track count for this artist.
    pub track_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
/// Album summary row returned by list endpoints.
pub struct AlbumSummary {
    /// Album id.
    pub id: i64,
    /// Stable album UUID.
    pub uuid: Option<String>,
    /// Album title.
    pub title: String,
    /// Album artist display name.
    pub artist: Option<String>,
    /// Album artist id.
    pub artist_id: Option<i64>,
    /// Display year.
    pub year: Option<i32>,
    /// Original release year.
    pub original_year: Option<i32>,
    /// Edition/reissue year.
    pub edition_year: Option<i32>,
    /// Edition label.
    pub edition_label: Option<String>,
    /// Optional MusicBrainz release MBID.
    pub mbid: Option<String>,
    /// Number of tracks in album.
    pub track_count: i64,
    /// Optional on-disk cover path.
    pub cover_art_path: Option<String>,
    /// Optional served cover URL.
    pub cover_art_url: Option<String>,
    /// True when album has at least one hi-res track.
    pub hi_res: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
/// Track summary row returned by list endpoints.
pub struct TrackSummary {
    /// Track id.
    pub id: i64,
    /// Filename for display.
    pub file_name: String,
    /// Track title.
    pub title: Option<String>,
    /// Track artist.
    pub artist: Option<String>,
    /// Album title.
    pub album: Option<String>,
    /// Track number.
    pub track_number: Option<u32>,
    /// Disc number.
    pub disc_number: Option<u32>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Format label.
    pub format: Option<String>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Bit depth.
    pub bit_depth: Option<u32>,
    /// Optional MusicBrainz recording MBID.
    pub mbid: Option<String>,
    /// Optional served cover URL.
    pub cover_art_url: Option<String>,
}

#[derive(Debug, Clone)]
/// Candidate album path used for writing album marker sidecars.
pub struct AlbumMarkerCandidate {
    /// Album UUID.
    pub album_uuid: String,
    /// Album title.
    pub title: Option<String>,
    /// Album artist.
    pub artist: Option<String>,
    /// Original release year.
    pub original_year: Option<i32>,
    /// Edition/display year.
    pub year: Option<i32>,
    /// Representative path for album folder.
    pub path: String,
}

#[derive(Debug, Clone)]
/// Text metadata value persisted for artist/album profiles.
pub struct TextEntry {
    /// Language tag.
    pub lang: String,
    /// Text payload.
    pub text: String,
    /// Optional source label/url.
    pub source: Option<String>,
    /// Locked flag for overwrite protection.
    pub locked: bool,
    /// Last update time (unix ms).
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
/// Media asset DB row for artist/album image records.
pub struct MediaAssetRecord {
    /// Media asset id.
    pub id: i64,
    /// Owner entity type (`artist`/`album`).
    pub owner_type: String,
    /// Owner entity id.
    pub owner_id: i64,
    /// Asset kind (for example `image`).
    pub kind: String,
    /// Local stored asset path.
    pub local_path: String,
    /// Optional checksum.
    pub checksum: Option<String>,
    /// Optional original source URL.
    pub source_url: Option<String>,
    /// Last update time (unix ms).
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
/// Track candidate for MusicBrainz enrichment jobs.
pub struct MusicBrainzCandidate {
    /// Track path.
    pub path: String,
    /// Track title.
    pub title: String,
    /// Track artist.
    pub artist: String,
    /// Album title.
    pub album: Option<String>,
    /// Album artist.
    pub album_artist: Option<String>,
    /// No-match key used to suppress repeated lookups.
    pub no_match_key: Option<String>,
}

#[derive(Debug, Clone)]
/// Album candidate for cover art enrichment jobs.
pub struct CoverArtCandidate {
    /// Album id.
    pub album_id: i64,
    /// MusicBrainz release MBID.
    pub mbid: String,
}

/// Map one SQL artist row into [`ArtistSummary`].
fn map_artist_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtistSummary> {
    Ok(ArtistSummary {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        sort_name: row.get(3)?,
        mbid: row.get(4)?,
        album_count: row.get(5)?,
        track_count: row.get(6)?,
    })
}

/// Map one SQL row into [`TextEntry`].
fn map_text_entry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TextEntry> {
    let locked: i64 = row.get(3)?;
    Ok(TextEntry {
        lang: row.get(0)?,
        text: row.get(1)?,
        source: row.get(2)?,
        locked: locked != 0,
        updated_at_ms: row.get(4)?,
    })
}

/// Map one SQL row into [`MediaAssetRecord`].
fn map_media_asset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaAssetRecord> {
    Ok(MediaAssetRecord {
        id: row.get(0)?,
        owner_type: row.get(1)?,
        owner_id: row.get(2)?,
        kind: row.get(3)?,
        local_path: row.get(4)?,
        checksum: row.get(5)?,
        source_url: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

impl MetadataDb {
    /// Open (or initialize) metadata DB under `<media_root>/.audio-hub/metadata.sqlite`.
    pub fn new(media_root: &Path) -> Result<Self> {
        let db_path = db_path_for(media_root);
        Self::new_at_path_with_media_root(&db_path, Some(media_root))
    }

    /// Open (or initialize) metadata DB at an explicit path.
    pub fn new_at_path(db_path: &Path) -> Result<Self> {
        Self::new_at_path_with_media_root(db_path, None)
    }

    /// Open DB at explicit path and optionally configure media root for path normalization.
    pub fn new_at_path_with_media_root(db_path: &Path, media_root: Option<&Path>) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create metadata dir {:?}", parent))?;
        }

        let manager = SqliteConnectionManager::file(&db_path).with_init(|conn| {
            conn.execute_batch("PRAGMA foreign_keys = ON;")?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .context("create metadata db pool")?;

        {
            let conn = pool.get().context("open metadata db")?;
            init_schema(&conn)?;
        }

        let db = Self {
            pool,
            media_root: media_root.map(normalize_media_root),
        };
        db.migrate_track_paths_to_relative()?;
        Ok(db)
    }

    /// Convert caller path into DB-stored path representation.
    fn path_to_db(&self, path: &str) -> String {
        let Some(root) = self.media_root.as_ref() else {
            return path.to_string();
        };
        let path_obj = Path::new(path);
        if !path_obj.is_absolute() {
            return path.to_string();
        }
        relative_from_absolute(path_obj, root)
            .map(|rel| rel.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    }

    /// Convert DB-stored path into caller-facing path representation.
    fn path_from_db(&self, path: String) -> String {
        let Some(root) = self.media_root.as_ref() else {
            return path;
        };
        let path_obj = Path::new(&path);
        if path_obj.is_absolute() {
            return path;
        }
        root.join(path_obj).to_string_lossy().to_string()
    }

    /// Migrate legacy absolute track paths into media-root-relative form.
    fn migrate_track_paths_to_relative(&self) -> Result<()> {
        let Some(root) = self.media_root.as_ref() else {
            return Ok(());
        };
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin path migration tx")?;
        let mut stmt = tx
            .prepare("SELECT id, path FROM tracks")
            .context("prepare path migration query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("run path migration query")?;
        let mut updates: Vec<(i64, String, String)> = Vec::new();
        for row in rows {
            let (id, existing_path) = row.context("map path migration row")?;
            let existing_obj = Path::new(&existing_path);
            if !existing_obj.is_absolute() {
                continue;
            }
            let Some(rel) = relative_from_absolute(existing_obj, root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().to_string();
            if rel_str != existing_path {
                updates.push((id, existing_path, rel_str));
            }
        }
        drop(stmt);

        for (id, old_path, rel_path) in updates {
            match tx.execute(
                "UPDATE tracks SET path = ?1 WHERE id = ?2",
                params![rel_path, id],
            ) {
                Ok(_) => {}
                Err(rusqlite::Error::SqliteFailure(err, _))
                    if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
                {
                    tx.execute("DELETE FROM tracks WHERE id = ?1", params![id])
                        .with_context(|| {
                            format!(
                                "drop duplicate track during path migration old={} new={}",
                                old_path, rel_path
                            )
                        })?;
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("migrate track path old={} new={}", old_path, rel_path)
                    });
                }
            }
        }
        tx.commit().context("commit path migration tx")?;
        Ok(())
    }

    /// Insert or update one track row and related artist/album rows.
    pub fn upsert_track(&self, record: &TrackRecord) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let record_path = self.path_to_db(&record.path);

        let existing: Option<(
            i64,
            i64,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<u32>,
            Option<u32>,
        )> = tx
            .query_row(
                r#"
                SELECT t.mtime_ms,
                       t.size_bytes,
                       t.artist_id,
                       t.album_id,
                       t.mbid,
                       ar.mbid,
                       al.mbid,
                       al.title,
                       aa.name,
                       t.disc_number,
                       t.bit_depth
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                LEFT JOIN artists aa ON aa.id = al.artist_id
                WHERE t.path = ?1
                "#,
                params![&record_path],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ))
                },
            )
            .optional()
            .context("lookup existing track")?;
        let (existing_artist_id, existing_album_id, keep_album_link) = if let Some((
            mtime_ms,
            size_bytes,
            artist_id,
            album_id,
            track_mbid,
            _artist_mbid,
            album_mbid,
            album_title,
            album_artist,
            disc_number,
            bit_depth,
        )) = existing
        {
            let album_title_same = match (record.album.as_deref(), album_title.as_deref()) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            let desired_album_artist = record.album_artist.as_deref().or(record.artist.as_deref());
            let album_artist_same = match (desired_album_artist, album_artist.as_deref()) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            let disc_same = record.disc_number == disc_number;
            let bit_depth_same = record.bit_depth == bit_depth;
            if mtime_ms == record.mtime_ms
                && size_bytes == record.size_bytes
                && album_title_same
                && album_artist_same
                && disc_same
                && bit_depth_same
            {
                return tx.commit().context("commit metadata tx");
            }
            let keep_album_link = !is_blank(&track_mbid) || !is_blank(&album_mbid);
            (artist_id, album_id, keep_album_link)
        } else {
            (None, None, false)
        };

        let artist_id = if let Some(name) = record.artist.as_deref() {
            Some(upsert_artist(&tx, name)?)
        } else {
            existing_artist_id
        };
        let album_artist_id =
            if let Some(name) = record.album_artist.as_deref().or(record.artist.as_deref()) {
                Some(upsert_artist(&tx, name)?)
            } else {
                None
            };
        let album_id = if keep_album_link {
            existing_album_id
        } else if let Some(title) = record.album.as_deref() {
            if let Some(uuid) = record.album_uuid.as_deref() {
                Some(upsert_album_with_uuid(
                    &tx,
                    uuid,
                    title,
                    album_artist_id,
                    record.year,
                )?)
            } else {
                Some(upsert_album(&tx, title, album_artist_id, record.year)?)
            }
        } else {
            None
        };

        tx.execute(
            r#"
            INSERT INTO tracks (
                path, file_name, title, artist_id, album_id, track_number, disc_number,
                duration_ms, sample_rate, bit_depth, format, mtime_ms, size_bytes, mb_no_match_key
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(path) DO UPDATE SET
                file_name = excluded.file_name,
                title = excluded.title,
                artist_id = excluded.artist_id,
                album_id = excluded.album_id,
                track_number = excluded.track_number,
                disc_number = excluded.disc_number,
                duration_ms = excluded.duration_ms,
                sample_rate = excluded.sample_rate,
                bit_depth = excluded.bit_depth,
                format = excluded.format,
                mtime_ms = excluded.mtime_ms,
                size_bytes = excluded.size_bytes,
                mb_no_match_key = NULL
            "#,
            params![
                &record_path,
                record.file_name,
                record.title,
                artist_id,
                album_id,
                record.track_number,
                record.disc_number,
                record.duration_ms.map(|v| v as i64),
                record.sample_rate.map(|v| v as i64),
                record.bit_depth.map(|v| v as i64),
                record.format,
                record.mtime_ms,
                record.size_bytes,
                Option::<String>::None
            ],
        )
        .context("upsert track")?;

        if let Some(album_id) = album_id {
            tx.execute(
                "UPDATE albums SET orphaned_at = NULL WHERE id = ?1",
                params![album_id],
            )
            .context("clear album orphaned_at")?;
        }

        tx.commit().context("commit metadata tx")?;
        Ok(())
    }

    /// Apply MusicBrainz metadata without overriding existing MB fields.
    pub fn apply_musicbrainz(&self, record: &TrackRecord, mb: &MusicBrainzMatch) -> Result<()> {
        self.apply_musicbrainz_with_override(record, mb, false)
    }

    /// Apply MusicBrainz metadata with optional overwrite behavior.
    pub fn apply_musicbrainz_with_override(
        &self,
        record: &TrackRecord,
        mb: &MusicBrainzMatch,
        override_existing: bool,
    ) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let record_path = self.path_to_db(&record.path);

        let artist_id = if let Some(name) = record.artist.as_deref() {
            find_artist_id(&tx, name)?
        } else {
            None
        };
        let album_id = tx
            .query_row(
                "SELECT album_id FROM tracks WHERE path = ?1",
                params![&record_path],
                |row| row.get(0),
            )
            .optional()
            .context("fetch album id for track")?;
        let album_id = if album_id.is_some() {
            album_id
        } else if let Some(title) = record.album.as_deref() {
            let album_artist_id =
                if let Some(name) = record.album_artist.as_deref().or(record.artist.as_deref()) {
                    find_artist_id(&tx, name)?
                } else {
                    None
                };
            find_album_id(&tx, title, album_artist_id)?
        } else {
            None
        };
        tracing::info!(
            path = %record.path,
            album_id = ?album_id,
            album = ?record.album,
            "apply musicbrainz (track) resolved album id"
        );

        if let (Some(artist_id), Some(artist_mbid)) = (artist_id, mb.artist_mbid.as_deref()) {
            if override_existing {
                tx.execute(
                    "UPDATE artists SET mbid = ?1 WHERE id = ?2",
                    params![artist_mbid, artist_id],
                )
                .context("update artist mbid")?;
            } else {
                tx.execute(
                    "UPDATE artists SET mbid = ?1 WHERE id = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![artist_mbid, artist_id],
                )
                .context("update artist mbid")?;
            }
            if let Some(sort_name) = mb.artist_sort_name.as_deref() {
                tx.execute(
                    "UPDATE artists SET sort_name = ?1 WHERE id = ?2 AND sort_name IS NULL",
                    params![sort_name, artist_id],
                )
                .context("update artist sort name")?;
            }
        }

        if let (Some(album_id), Some(album_mbid)) = (album_id, mb.album_mbid.as_deref()) {
            if override_existing {
                let updated = tx.execute(
                    "UPDATE albums SET mbid = ?1, cover_art_path = NULL, caa_fail_count = NULL, caa_last_error = NULL, caa_release_candidates = NULL WHERE id = ?2",
                    params![album_mbid, album_id],
                )
                .context("update album mbid")?;
                tracing::info!(album_id, updated, "apply musicbrainz (track) updated album");
            } else {
                let updated = tx.execute(
                    "UPDATE albums SET mbid = ?1, caa_fail_count = NULL, caa_last_error = NULL, caa_release_candidates = NULL WHERE id = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![album_mbid, album_id],
                )
                .context("update album mbid")?;
                tracing::info!(album_id, updated, "apply musicbrainz (track) updated album");
            }
            if let Some(year) = mb.release_year {
                if override_existing {
                    tx.execute(
                        "UPDATE albums SET year = ?1 WHERE id = ?2",
                        params![year, album_id],
                    )
                    .context("update album year")?;
                } else {
                    tx.execute(
                        "UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL",
                        params![year, album_id],
                    )
                    .context("update album year")?;
                }
            }
            if !mb.release_candidates.is_empty() {
                let candidates = serde_json::to_string(&mb.release_candidates)?;
                tx.execute(
                    "UPDATE albums SET caa_release_candidates = ?1 WHERE id = ?2",
                    params![candidates, album_id],
                )
                .context("update album release candidates")?;
            }
        }

        if let Some(recording_mbid) = mb.recording_mbid.as_deref() {
            if override_existing {
                tx.execute(
                    "UPDATE tracks SET mbid = ?1, mb_no_match_key = NULL WHERE path = ?2",
                    params![recording_mbid, &record_path],
                )
                .context("update track mbid")?;
            } else {
                tx.execute(
                    "UPDATE tracks SET mbid = ?1, mb_no_match_key = NULL WHERE path = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![recording_mbid, &record_path],
                )
                .context("update track mbid")?;
            }
        }

        tx.commit().context("commit metadata tx")?;
        Ok(())
    }

    /// Apply album-scoped MusicBrainz metadata updates.
    pub fn apply_album_musicbrainz(
        &self,
        album_id: i64,
        mb: &MusicBrainzMatch,
        override_existing: bool,
    ) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let artist_id: Option<i64> = tx
            .query_row(
                "SELECT artist_id FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch album artist id")?;

        if let (Some(artist_id), Some(artist_mbid)) = (artist_id, mb.artist_mbid.as_deref()) {
            if override_existing {
                tx.execute(
                    "UPDATE artists SET mbid = ?1 WHERE id = ?2",
                    params![artist_mbid, artist_id],
                )
                .context("update artist mbid")?;
            } else {
                tx.execute(
                    "UPDATE artists SET mbid = ?1 WHERE id = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![artist_mbid, artist_id],
                )
                .context("update artist mbid")?;
            }
        }

        if let Some(album_mbid) = mb.album_mbid.as_deref() {
            if override_existing {
                let updated = tx.execute(
                    "UPDATE albums SET mbid = ?1, cover_art_path = NULL, caa_fail_count = NULL, caa_last_error = NULL, caa_release_candidates = NULL WHERE id = ?2",
                    params![album_mbid, album_id],
                )
                .context("update album mbid")?;
                tracing::info!(album_id, updated, "apply musicbrainz (album) updated album");
            } else {
                let updated = tx.execute(
                    "UPDATE albums SET mbid = ?1, caa_fail_count = NULL, caa_last_error = NULL, caa_release_candidates = NULL WHERE id = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![album_mbid, album_id],
                )
                .context("update album mbid")?;
                tracing::info!(album_id, updated, "apply musicbrainz (album) updated album");
            }
        }

        if let Some(year) = mb.release_year {
            if override_existing {
                tx.execute(
                    "UPDATE albums SET year = ?1 WHERE id = ?2",
                    params![year, album_id],
                )
                .context("update album year")?;
            } else {
                tx.execute(
                    "UPDATE albums SET year = ?1 WHERE id = ?2 AND year IS NULL",
                    params![year, album_id],
                )
                .context("update album year")?;
            }
        }

        tx.commit().context("commit metadata tx")?;
        Ok(())
    }

    /// Fetch full track record by path.
    pub fn track_record_by_path(&self, path: &str) -> Result<Option<TrackRecord>> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        conn.query_row(
            r#"
                SELECT t.path, t.file_name, t.title, ar.name, aa.name, al.title, al.uuid,
                       t.track_number, t.disc_number, al.year, t.duration_ms,
                       t.sample_rate, t.bit_depth, t.format, t.mtime_ms, t.size_bytes
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                LEFT JOIN artists aa ON aa.id = al.artist_id
                WHERE t.path = ?1
                "#,
            params![db_path],
            |row| {
                let path: String = row.get(0)?;
                Ok(TrackRecord {
                    path: self.path_from_db(path),
                    file_name: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    album_artist: row.get(4)?,
                    album: row.get(5)?,
                    album_uuid: row.get(6)?,
                    track_number: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                    disc_number: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                    year: row.get(9)?,
                    duration_ms: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    sample_rate: row.get::<_, Option<i64>>(11)?.map(|v| v as u32),
                    bit_depth: row.get::<_, Option<i64>>(12)?.map(|v| v as u32),
                    format: row.get(13)?,
                    mtime_ms: row.get(14)?,
                    size_bytes: row.get(15)?,
                })
            },
        )
        .optional()
        .context("fetch track record")
    }

    /// Fetch full track record by id.
    pub fn track_record_by_id(&self, track_id: i64) -> Result<Option<TrackRecord>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
                SELECT t.path, t.file_name, t.title, ar.name, aa.name, al.title, al.uuid,
                       t.track_number, t.disc_number, al.year, t.duration_ms,
                       t.sample_rate, t.bit_depth, t.format, t.mtime_ms, t.size_bytes
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                LEFT JOIN artists aa ON aa.id = al.artist_id
                WHERE t.id = ?1
                "#,
            params![track_id],
            |row| {
                let path: String = row.get(0)?;
                Ok(TrackRecord {
                    path: self.path_from_db(path),
                    file_name: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    album_artist: row.get(4)?,
                    album: row.get(5)?,
                    album_uuid: row.get(6)?,
                    track_number: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                    disc_number: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                    year: row.get(9)?,
                    duration_ms: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    sample_rate: row.get::<_, Option<i64>>(11)?.map(|v| v as u32),
                    bit_depth: row.get::<_, Option<i64>>(12)?.map(|v| v as u32),
                    format: row.get(13)?,
                    mtime_ms: row.get(14)?,
                    size_bytes: row.get(15)?,
                })
            },
        )
        .optional()
        .context("fetch track record")
    }

    /// Resolve track id by path.
    pub fn track_id_for_path(&self, path: &str) -> Result<Option<i64>> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        conn.query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            params![db_path],
            |row| row.get(0),
        )
        .optional()
        .context("fetch track id for path")
    }

    /// Resolve track path by track id.
    pub fn track_path_for_id(&self, track_id: i64) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let path: Option<String> = conn
            .query_row(
                "SELECT path FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch track path for id")?;
        Ok(path.map(|value| self.path_from_db(value)))
    }

    /// Resolve album id containing a given track path.
    pub fn album_id_for_track_path(&self, path: &str) -> Result<Option<i64>> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        conn.query_row(
            "SELECT album_id FROM tracks WHERE path = ?1",
            params![db_path],
            |row| row.get(0),
        )
        .optional()
        .context("fetch album id for track")
    }

    /// List representative album-path candidates for marker file generation.
    pub fn album_marker_candidates(&self) -> Result<Vec<AlbumMarkerCandidate>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare(
            r#"
            SELECT al.uuid, al.title, ar.name, al.original_year, al.year, MIN(t.path)
            FROM albums al
            JOIN tracks t ON t.album_id = al.id
            LEFT JOIN artists ar ON ar.id = al.artist_id
            WHERE al.uuid IS NOT NULL AND al.uuid != ''
            GROUP BY al.id
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(5)?;
            Ok(AlbumMarkerCandidate {
                album_uuid: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                original_year: row.get(3)?,
                year: row.get(4)?,
                path: self.path_from_db(path),
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Lookup album UUID by normalized title and optional artist.
    pub fn album_uuid_for_title_artist(
        &self,
        title: &str,
        artist: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let artist_id = if let Some(name) = artist {
            find_artist_id(&conn, name)?
        } else {
            None
        };
        conn.query_row(
            "SELECT uuid FROM albums WHERE title = ?1 AND artist_id IS ?2",
            params![title, artist_id],
            |row| row.get(0),
        )
        .optional()
        .context("lookup album uuid by title/artist")
    }

    /// List tracks still missing at least one MusicBrainz binding.
    pub fn list_musicbrainz_candidates(&self, limit: i64) -> Result<Vec<MusicBrainzCandidate>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare(
            r#"
            SELECT t.path, t.title, ar.name, al.title, aa.name, t.mb_no_match_key
            FROM tracks t
            LEFT JOIN artists ar ON ar.id = t.artist_id
            LEFT JOIN albums al ON al.id = t.album_id
            LEFT JOIN artists aa ON aa.id = al.artist_id
            WHERE t.title IS NOT NULL
              AND ar.name IS NOT NULL
              AND (
                t.mbid IS NULL OR t.mbid = ''
                OR ar.mbid IS NULL OR ar.mbid = ''
                OR al.mbid IS NULL OR al.mbid = ''
              )
            ORDER BY t.path
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            let path: String = row.get(0)?;
            Ok(MusicBrainzCandidate {
                path: self.path_from_db(path),
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                album_artist: row.get(4)?,
                no_match_key: row.get(5)?,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Lookup album cover path by title/optional artist.
    pub fn album_cover_path(&self, album: &str, artist: Option<&str>) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let artist_id = if let Some(artist) = artist {
            find_artist_id(&conn, artist)?
        } else {
            None
        };
        let album_id = find_album_id(&conn, album, artist_id)?;
        let Some(album_id) = album_id else {
            return Ok(None);
        };
        let cover: Option<String> = conn
            .query_row(
                "SELECT cover_art_path FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch album cover path")?;
        Ok(cover.filter(|value| !value.trim().is_empty()))
    }

    /// Set album cover path only when existing cover field is empty.
    pub fn set_album_cover_if_empty(
        &self,
        album: &str,
        artist: Option<&str>,
        cover_path: &str,
    ) -> Result<bool> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let artist_id = if let Some(artist) = artist {
            find_artist_id(&tx, artist)?
        } else {
            None
        };
        let album_id = find_album_id(&tx, album, artist_id)?;
        let Some(album_id) = album_id else {
            return Ok(false);
        };
        let updated = tx.execute(
            "UPDATE albums SET cover_art_path = ?1 WHERE id = ?2 AND (cover_art_path IS NULL OR cover_art_path = '')",
            params![cover_path, album_id],
        )?;
        tx.commit().context("commit metadata tx")?;
        Ok(updated > 0)
    }

    /// Lookup cover path for one track path.
    pub fn cover_path_for_track(&self, path: &str) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        let cover: Option<String> = conn
            .query_row(
                r#"
                SELECT al.cover_art_path
                FROM tracks t
                LEFT JOIN albums al ON al.id = t.album_id
                WHERE t.path = ?1
                "#,
                params![db_path],
                |row| row.get(0),
            )
            .optional()
            .context("fetch cover path for track")?;
        Ok(cover.filter(|value| !value.trim().is_empty()))
    }

    /// Lookup cover path for one track id.
    pub fn cover_path_for_track_id(&self, track_id: i64) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let cover: Option<String> = conn
            .query_row(
                r#"
                SELECT al.cover_art_path
                FROM tracks t
                LEFT JOIN albums al ON al.id = t.album_id
                WHERE t.id = ?1
                "#,
                params![track_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch cover path for track id")?;
        Ok(cover.filter(|value| !value.trim().is_empty()))
    }

    /// Lookup cover path for one album id.
    pub fn cover_path_for_album_id(&self, album_id: i64) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let cover: Option<String> = conn
            .query_row(
                "SELECT cover_art_path FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch cover path for album")?;
        Ok(cover.filter(|value| !value.trim().is_empty()))
    }

    /// Set album cover for album id only when currently empty.
    pub fn set_album_cover_by_id_if_empty(&self, album_id: i64, cover_path: &str) -> Result<bool> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let updated = tx.execute(
            "UPDATE albums SET cover_art_path = ?1, caa_fail_count = NULL, caa_last_error = NULL WHERE id = ?2 AND (cover_art_path IS NULL OR cover_art_path = '')",
            params![cover_path, album_id],
        )?;
        tx.commit().context("commit metadata tx")?;
        Ok(updated > 0)
    }

    /// List albums eligible for cover-art fetch attempts.
    pub fn list_cover_art_candidates(&self, limit: i64) -> Result<Vec<CoverArtCandidate>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare(
            r#"
            SELECT al.id, al.mbid, COALESCE(al.caa_fail_count, 0), al.caa_release_candidates
            FROM albums al
            WHERE al.mbid IS NOT NULL
              AND al.mbid != ''
              AND (al.cover_art_path IS NULL OR al.cover_art_path = '')
              AND COALESCE(al.caa_fail_count, 0) < 3
            ORDER BY al.id
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(CoverArtCandidate {
                album_id: row.get(0)?,
                mbid: row.get(1)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Increment cover-art failure count and persist last error text.
    pub fn increment_cover_art_fail(&self, album_id: i64, error: &str) -> Result<i64> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            "UPDATE albums SET caa_fail_count = COALESCE(caa_fail_count, 0) + 1, caa_last_error = ?1 WHERE id = ?2",
            params![error, album_id],
        )
        .context("increment cover art fail count")?;
        let count: i64 = conn.query_row(
            "SELECT COALESCE(caa_fail_count, 0) FROM albums WHERE id = ?1",
            params![album_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Advance album to next fallback release MBID candidate.
    pub fn advance_cover_candidate(&self, album_id: i64) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT caa_release_candidates FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .context("fetch release candidates")?;
        let mut candidates: Vec<String> = raw
            .as_deref()
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .unwrap_or_default();
        let Some(next) = candidates.first().cloned() else {
            return Ok(None);
        };
        candidates.remove(0);
        let updated_candidates = if candidates.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&candidates)?)
        };
        conn.execute(
            "UPDATE albums SET mbid = ?1, caa_release_candidates = ?2, caa_fail_count = NULL, caa_last_error = NULL WHERE id = ?3",
            params![next, updated_candidates, album_id],
        )
        .context("advance cover candidate")?;
        Ok(Some(next))
    }

    /// Persist no-match key for a track to suppress repeated MB lookups.
    pub fn set_musicbrainz_no_match(&self, path: &str, key: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        conn.execute(
            "UPDATE tracks SET mb_no_match_key = ?1 WHERE path = ?2",
            params![key, db_path],
        )
        .context("set musicbrainz no match")?;
        Ok(())
    }

    /// Clear stored no-match suppression key for a track.
    pub fn clear_musicbrainz_no_match(&self, path: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        conn.execute(
            "UPDATE tracks SET mb_no_match_key = NULL WHERE path = ?1",
            params![db_path],
        )
        .context("clear musicbrainz no match")?;
        Ok(())
    }

    /// List artist summaries with optional search and paging.
    pub fn list_artists(
        &self,
        search: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ArtistSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        let search_like = search.map(|s| format!("%{}%", s.to_lowercase()));
        let mut stmt = if search_like.is_some() {
            conn.prepare(
                r#"
            SELECT a.id, a.uuid, a.name, a.sort_name, a.mbid,
                   COUNT(DISTINCT al.id) AS album_count,
                   COUNT(t.id) AS track_count
                FROM artists a
                LEFT JOIN albums al ON al.artist_id = a.id
                LEFT JOIN tracks t ON t.artist_id = a.id
                WHERE LOWER(a.name) LIKE ?1
                GROUP BY a.id
                ORDER BY a.name
                LIMIT ?2 OFFSET ?3
                "#,
            )?
        } else {
            conn.prepare(
                r#"
            SELECT a.id, a.uuid, a.name, a.sort_name, a.mbid,
                   COUNT(DISTINCT al.id) AS album_count,
                   COUNT(t.id) AS track_count
                FROM artists a
                LEFT JOIN albums al ON al.artist_id = a.id
                LEFT JOIN tracks t ON t.artist_id = a.id
                GROUP BY a.id
                ORDER BY a.name
                LIMIT ?1 OFFSET ?2
                "#,
            )?
        };

        let rows = if let Some(search_like) = search_like {
            stmt.query_map(params![search_like, limit, offset], map_artist_row)?
        } else {
            stmt.query_map(params![limit, offset], map_artist_row)?
        };

        Ok(rows.filter_map(Result::ok).collect())
    }

    /// List album summaries with optional artist/search filters and paging.
    pub fn list_albums(
        &self,
        artist_id: Option<i64>,
        search: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AlbumSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        let search_like = search.map(|s| format!("%{}%", s.to_lowercase()));
        let mut stmt = conn.prepare(
            r#"
            SELECT al.id, al.uuid, al.title, ar.name, al.artist_id, al.year,
                   al.original_year, al.edition_year, al.edition_label, al.mbid,
                   COUNT(t.id) AS track_count, al.cover_art_path,
                   MAX(t.bit_depth) AS max_bit_depth
            FROM albums al
            LEFT JOIN artists ar ON ar.id = al.artist_id
            LEFT JOIN tracks t ON t.album_id = al.id
            WHERE (?1 IS NULL OR al.artist_id = ?1)
              AND (?2 IS NULL OR LOWER(al.title) LIKE ?2)
              AND al.orphaned_at IS NULL
            GROUP BY al.id
            ORDER BY
                CASE WHEN ar.name IS NULL THEN 1 ELSE 0 END,
                COALESCE(ar.sort_name, ar.name),
                COALESCE(al.original_year, al.year, 9999),
                COALESCE(al.sort_title, al.title)
            LIMIT ?3 OFFSET ?4
            "#,
        )?;
        let rows = stmt.query_map(params![artist_id, search_like, limit, offset], |row| {
            let album_id: i64 = row.get(0)?;
            let cover_path: Option<String> = row.get(11)?;
            let max_bit_depth: Option<i64> = row.get(12)?;
            let hi_res = max_bit_depth.unwrap_or(0) >= 24;
            let cover_art_url = cover_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|_| format!("/albums/{}/cover", album_id));
            Ok(AlbumSummary {
                id: album_id,
                uuid: row.get(1)?,
                title: row.get(2)?,
                artist: row.get(3)?,
                artist_id: row.get(4)?,
                year: row.get(5)?,
                original_year: row.get(6)?,
                edition_year: row.get(7)?,
                edition_label: row.get(8)?,
                mbid: row.get(9)?,
                track_count: row.get(10)?,
                cover_art_path: cover_path,
                cover_art_url,
                hi_res,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Fetch one album summary by id.
    pub fn album_summary_by_id(&self, album_id: i64) -> Result<Option<AlbumSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
                SELECT al.id, al.uuid, al.title, ar.name, al.artist_id, al.year,
                       al.original_year, al.edition_year, al.edition_label, al.mbid,
                       COUNT(t.id) AS track_count, al.cover_art_path,
                       MAX(t.bit_depth) AS max_bit_depth
                FROM albums al
                LEFT JOIN artists ar ON ar.id = al.artist_id
                LEFT JOIN tracks t ON t.album_id = al.id
                WHERE al.id = ?1
                GROUP BY al.id
                "#,
            params![album_id],
            |row| {
                let album_id: i64 = row.get(0)?;
                let cover_path: Option<String> = row.get(11)?;
                let max_bit_depth: Option<i64> = row.get(12)?;
                let hi_res = max_bit_depth.unwrap_or(0) >= 24;
                let cover_art_url = cover_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| format!("/albums/{}/cover", album_id));
                Ok(AlbumSummary {
                    id: album_id,
                    uuid: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    artist_id: row.get(4)?,
                    year: row.get(5)?,
                    original_year: row.get(6)?,
                    edition_year: row.get(7)?,
                    edition_label: row.get(8)?,
                    mbid: row.get(9)?,
                    track_count: row.get(10)?,
                    cover_art_path: cover_path,
                    cover_art_url,
                    hi_res,
                })
            },
        )
        .optional()
        .context("select album summary by id")
    }

    /// Return whether an artist row exists.
    pub fn artist_exists(&self, artist_id: i64) -> Result<bool> {
        let conn = self.pool.get().context("open metadata db")?;
        let value: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM artists WHERE id = ?1",
                params![artist_id],
                |row| row.get(0),
            )
            .optional()
            .context("select artist exists")?;
        Ok(value.is_some())
    }

    /// Return whether an album row exists.
    pub fn album_exists(&self, album_id: i64) -> Result<bool> {
        let conn = self.pool.get().context("open metadata db")?;
        let value: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM albums WHERE id = ?1",
                params![album_id],
                |row| row.get(0),
            )
            .optional()
            .context("select album exists")?;
        Ok(value.is_some())
    }

    /// Fetch album edition fields tuple `(original_year, edition_year, edition_label)`.
    pub fn album_edition_fields(
        &self,
        album_id: i64,
    ) -> Result<(Option<i32>, Option<i32>, Option<String>)> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            "SELECT original_year, edition_year, edition_label FROM albums WHERE id = ?1",
            params![album_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("select album edition fields")
    }

    /// Update album edition fields.
    pub fn update_album_edition_fields(
        &self,
        album_id: i64,
        original_year: Option<i32>,
        edition_year: Option<i32>,
        edition_label: Option<&str>,
    ) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            r#"
            UPDATE albums
            SET original_year = ?1,
                edition_year = ?2,
                edition_label = ?3
            WHERE id = ?4
            "#,
            params![original_year, edition_year, edition_label, album_id],
        )
        .context("update album edition fields")?;
        Ok(())
    }

    /// Fetch artist biography entry for `(artist_id, lang)`.
    pub fn artist_bio(&self, artist_id: i64, lang: &str) -> Result<Option<TextEntry>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
            SELECT lang, text, source, locked, updated_at_ms
            FROM artist_bios
            WHERE artist_id = ?1 AND lang = ?2
            "#,
            params![artist_id, lang],
            map_text_entry_row,
        )
        .optional()
        .context("select artist bio")
    }

    /// Fetch album notes entry for `(album_id, lang)`.
    pub fn album_notes(&self, album_id: i64, lang: &str) -> Result<Option<TextEntry>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
            SELECT lang, text, source, locked, updated_at_ms
            FROM album_notes
            WHERE album_id = ?1 AND lang = ?2
            "#,
            params![album_id, lang],
            map_text_entry_row,
        )
        .optional()
        .context("select album notes")
    }

    /// Insert or update one artist biography entry.
    pub fn upsert_artist_bio(
        &self,
        artist_id: i64,
        lang: &str,
        text: &str,
        source: Option<&str>,
        locked: bool,
        updated_at_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        let locked_value = if locked { 1 } else { 0 };
        conn.execute(
            r#"
            INSERT INTO artist_bios (artist_id, lang, text, source, locked, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(artist_id, lang) DO UPDATE SET
                text = excluded.text,
                source = excluded.source,
                locked = excluded.locked,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![artist_id, lang, text, source, locked_value, updated_at_ms],
        )
        .context("upsert artist bio")?;
        Ok(())
    }

    /// Insert or update one album notes entry.
    pub fn upsert_album_notes(
        &self,
        album_id: i64,
        lang: &str,
        text: &str,
        source: Option<&str>,
        locked: bool,
        updated_at_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        let locked_value = if locked { 1 } else { 0 };
        conn.execute(
            r#"
            INSERT INTO album_notes (album_id, lang, text, source, locked, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(album_id, lang) DO UPDATE SET
                text = excluded.text,
                source = excluded.source,
                locked = excluded.locked,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![album_id, lang, text, source, locked_value, updated_at_ms],
        )
        .context("upsert album notes")?;
        Ok(())
    }

    /// Delete artist biography for `(artist_id, lang)`.
    pub fn delete_artist_bio(&self, artist_id: i64, lang: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            "DELETE FROM artist_bios WHERE artist_id = ?1 AND lang = ?2",
            params![artist_id, lang],
        )
        .context("delete artist bio")?;
        Ok(())
    }

    /// Delete album notes for `(album_id, lang)`.
    pub fn delete_album_notes(&self, album_id: i64, lang: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            "DELETE FROM album_notes WHERE album_id = ?1 AND lang = ?2",
            params![album_id, lang],
        )
        .context("delete album notes")?;
        Ok(())
    }

    /// Fetch media asset by owner tuple `(type, id, kind)`.
    pub fn media_asset_for(
        &self,
        owner_type: &str,
        owner_id: i64,
        kind: &str,
    ) -> Result<Option<MediaAssetRecord>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
            SELECT id, owner_type, owner_id, kind, local_path, checksum, source_url, updated_at_ms
            FROM media_assets
            WHERE owner_type = ?1 AND owner_id = ?2 AND kind = ?3
            "#,
            params![owner_type, owner_id, kind],
            map_media_asset_row,
        )
        .optional()
        .context("select media asset")
    }

    /// Fetch media asset by asset id.
    pub fn media_asset_by_id(&self, asset_id: i64) -> Result<Option<MediaAssetRecord>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.query_row(
            r#"
            SELECT id, owner_type, owner_id, kind, local_path, checksum, source_url, updated_at_ms
            FROM media_assets
            WHERE id = ?1
            "#,
            params![asset_id],
            map_media_asset_row,
        )
        .optional()
        .context("select media asset by id")
    }

    /// Replace existing media asset for owner tuple and return inserted asset id.
    pub fn upsert_media_asset(
        &self,
        owner_type: &str,
        owner_id: i64,
        kind: &str,
        local_path: &str,
        checksum: Option<&str>,
        source_url: Option<&str>,
        updated_at_ms: Option<i64>,
    ) -> Result<i64> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin media asset tx")?;
        tx.execute(
            "DELETE FROM media_assets WHERE owner_type = ?1 AND owner_id = ?2 AND kind = ?3",
            params![owner_type, owner_id, kind],
        )
        .context("delete existing media asset")?;
        tx.execute(
            r#"
            INSERT INTO media_assets (owner_type, owner_id, kind, local_path, checksum, source_url, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                owner_type,
                owner_id,
                kind,
                local_path,
                checksum,
                source_url,
                updated_at_ms
            ],
        )
        .context("insert media asset")?;
        let id = tx.last_insert_rowid();
        tx.commit().context("commit media asset tx")?;
        Ok(id)
    }

    /// Delete media asset for owner tuple and return previous row when present.
    pub fn delete_media_asset(
        &self,
        owner_type: &str,
        owner_id: i64,
        kind: &str,
    ) -> Result<Option<MediaAssetRecord>> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin delete media asset tx")?;
        let existing = tx
            .query_row(
                r#"
                SELECT id, owner_type, owner_id, kind, local_path, checksum, source_url, updated_at_ms
                FROM media_assets
                WHERE owner_type = ?1 AND owner_id = ?2 AND kind = ?3
                "#,
                params![owner_type, owner_id, kind],
                map_media_asset_row,
            )
            .optional()
            .context("select existing media asset")?;
        tx.execute(
            "DELETE FROM media_assets WHERE owner_type = ?1 AND owner_id = ?2 AND kind = ?3",
            params![owner_type, owner_id, kind],
        )
        .context("delete media asset")?;
        tx.commit().context("commit delete media asset tx")?;
        Ok(existing)
    }

    /// List tracks with optional album/artist/search filters and paging.
    pub fn list_tracks(
        &self,
        album_id: Option<i64>,
        artist_id: Option<i64>,
        search: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TrackSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        let search_like = search.map(|s| format!("%{}%", s.to_lowercase()));
        let mut stmt = conn.prepare(
            r#"
            SELECT t.id, t.file_name, t.title, ar.name, al.title,
                   t.track_number, t.disc_number, t.duration_ms, t.format,
                   t.sample_rate, t.bit_depth, t.mbid, al.cover_art_path
            FROM tracks t
            LEFT JOIN artists ar ON ar.id = t.artist_id
            LEFT JOIN albums al ON al.id = t.album_id
            WHERE (?1 IS NULL OR t.album_id = ?1)
              AND (?2 IS NULL OR t.artist_id = ?2)
              AND (?3 IS NULL OR LOWER(COALESCE(t.title, t.file_name)) LIKE ?3)
            ORDER BY COALESCE(t.disc_number, 0), COALESCE(t.track_number, 0), t.file_name
            LIMIT ?4 OFFSET ?5
            "#,
        )?;
        let rows = stmt.query_map(
            params![album_id, artist_id, search_like, limit, offset],
            |row| {
                let track_id: i64 = row.get(0)?;
                let cover_path: Option<String> = row.get(12)?;
                let cover_art_url = cover_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| format!("/tracks/{}/cover", track_id));
                Ok(TrackSummary {
                    id: track_id,
                    file_name: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    album: row.get(4)?,
                    track_number: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                    disc_number: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
                    duration_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                    format: row.get(8)?,
                    sample_rate: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
                    bit_depth: row.get::<_, Option<i64>>(10)?.map(|v| v as u32),
                    mbid: row.get(11)?,
                    cover_art_url,
                })
            },
        )?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    /// List track paths belonging to an album id.
    pub fn list_track_paths_by_album_id(&self, album_id: i64) -> Result<Vec<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare(
            "SELECT path FROM tracks WHERE album_id = ?1 ORDER BY COALESCE(disc_number, 0), COALESCE(track_number, 0), file_name",
        )?;
        let rows = stmt.query_map(params![album_id], |row| row.get(0))?;
        Ok(rows
            .filter_map(Result::ok)
            .map(|path: String| self.path_from_db(path))
            .collect())
    }

    /// List all track paths currently in DB.
    pub fn list_all_track_paths(&self) -> Result<Vec<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare("SELECT path FROM tracks")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows
            .filter_map(Result::ok)
            .map(|path: String| self.path_from_db(path))
            .collect())
    }

    /// Update album metadata, merging rows when title+artist collide.
    pub fn update_album_metadata(
        &self,
        album_id: i64,
        title: Option<&str>,
        artist: Option<&str>,
        year: Option<i32>,
    ) -> Result<Option<i64>> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let current: Option<(String, Option<i64>)> = tx
            .query_row(
                "SELECT title, artist_id FROM albums WHERE id = ?1",
                params![album_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("lookup current album")?;
        let Some((current_title, current_artist_id)) = current else {
            return Ok(None);
        };
        let artist_id = if let Some(name) = artist {
            Some(upsert_artist(&tx, name)?)
        } else {
            None
        };
        let desired_title = title.unwrap_or(current_title.as_str());
        let desired_artist_id = artist_id.or(current_artist_id);
        if let Some(existing_id) = find_album_id(&tx, desired_title, desired_artist_id)? {
            if existing_id != album_id {
                tx.execute(
                    r#"
                    UPDATE albums
                    SET title = ?1,
                        sort_title = LOWER(?1),
                        artist_id = ?2,
                        year = COALESCE(?3, year)
                    WHERE id = ?4
                    "#,
                    params![desired_title, desired_artist_id, year, existing_id],
                )
                .context("merge album metadata")?;
                tx.execute(
                    "UPDATE tracks SET album_id = ?1 WHERE album_id = ?2",
                    params![existing_id, album_id],
                )
                .context("reassign tracks to merged album")?;
                tx.execute("DELETE FROM albums WHERE id = ?1", params![album_id])
                    .context("delete merged album")?;
                tx.commit().context("commit metadata tx")?;
                return Ok(Some(existing_id));
            }
        }
        let updated = tx
            .execute(
                r#"
                UPDATE albums
                SET title = COALESCE(?1, title),
                    sort_title = CASE WHEN ?1 IS NULL THEN sort_title ELSE LOWER(?1) END,
                    artist_id = COALESCE(?2, artist_id),
                    year = COALESCE(?3, year)
                WHERE id = ?4
                "#,
                params![title, artist_id, year, album_id],
            )
            .context("update album metadata")?;
        tx.commit().context("commit metadata tx")?;
        if updated > 0 {
            Ok(Some(album_id))
        } else {
            Ok(None)
        }
    }

    /// Delete one track by path.
    pub fn delete_track_by_path(&self, path: &str) -> Result<bool> {
        let conn = self.pool.get().context("open metadata db")?;
        let db_path = self.path_to_db(path);
        let deleted = conn
            .execute("DELETE FROM tracks WHERE path = ?1", params![db_path])
            .context("delete track by path")?;
        Ok(deleted > 0)
    }

    /// Mark/clear orphaned albums according to current track references.
    pub fn prune_orphaned_albums_and_artists(&self) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        tx.execute(
            r#"
            UPDATE albums
            SET orphaned_at = ?1
            WHERE orphaned_at IS NULL
              AND id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)
            "#,
            params![now_ms],
        )
        .context("mark orphaned albums")?;
        tx.execute(
            r#"
            UPDATE albums
            SET orphaned_at = NULL
            WHERE orphaned_at IS NOT NULL
              AND id IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)
            "#,
            [],
        )
        .context("clear orphaned albums")?;
        tx.commit().context("commit metadata tx")?;
        Ok(())
    }
}

/// Compute canonical DB path under media root.
fn db_path_for(media_root: &Path) -> PathBuf {
    media_root.join(".audio-hub").join("metadata.sqlite")
}

/// Best-effort canonicalized media root for path normalization.
fn normalize_media_root(media_root: &Path) -> PathBuf {
    media_root
        .canonicalize()
        .unwrap_or_else(|_| media_root.to_path_buf())
}

/// Convert absolute path into media-root-relative path when possible.
fn relative_from_absolute(path: &Path, media_root: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(media_root) {
        return Some(relative.to_path_buf());
    }
    let media_name = media_root.file_name()?;
    let mut matched_index: Option<usize> = None;
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate() {
        if component.as_os_str() == media_name {
            matched_index = Some(index);
        }
    }
    let start = matched_index?;
    let relative = components
        .iter()
        .skip(start + 1)
        .fold(PathBuf::new(), |mut acc, component| {
            acc.push(component.as_os_str());
            acc
        });
    if relative.as_os_str().is_empty() {
        return None;
    }
    if media_root.join(&relative).exists() {
        Some(relative)
    } else {
        None
    }
}

/// Return true when optional string is missing or whitespace-only.
fn is_blank(value: &Option<String>) -> bool {
    value
        .as_deref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
}

/// Fill missing UUID values for all rows in a table.
fn backfill_uuids(conn: &Connection, table: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("SELECT id FROM {table} WHERE uuid IS NULL"))?;
    let ids = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    for id in ids.filter_map(Result::ok) {
        let uuid = Uuid::new_v4().to_string();
        conn.execute(
            &format!("UPDATE {table} SET uuid = ?1 WHERE id = ?2"),
            params![uuid, id],
        )
        .with_context(|| format!("backfill {table} uuid"))?;
    }
    Ok(())
}

/// Ensure UUID unique indexes exist on artists/albums.
fn ensure_uuid_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_artists_uuid ON artists(uuid);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_albums_uuid ON albums(uuid);
        "#,
    )
    .context("create uuid indexes")?;
    Ok(())
}

/// Ensure one row has a non-empty UUID value.
fn ensure_row_uuid(conn: &Connection, table: &str, id: i64) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            &format!("SELECT uuid FROM {table} WHERE id = ?1"),
            params![id],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| format!("select {table} uuid"))?;
    if existing
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        let uuid = Uuid::new_v4().to_string();
        conn.execute(
            &format!("UPDATE {table} SET uuid = ?1 WHERE id = ?2"),
            params![uuid, id],
        )
        .with_context(|| format!("update {table} uuid"))?;
    }
    Ok(())
}

/// Lookup artist id by exact artist name.
fn find_artist_id(conn: &Connection, name: &str) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM artists WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )
        .optional()
        .context("find artist id")?;
    Ok(id)
}

/// Lookup album id by exact `(title, artist_id)` pair.
fn find_album_id(conn: &Connection, title: &str, artist_id: Option<i64>) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM albums WHERE title = ?1 AND artist_id IS ?2",
            params![title, artist_id],
            |row| row.get(0),
        )
        .optional()
        .context("find album id")?;
    Ok(id)
}

/// Initialize/migrate metadata schema to current version.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artists (
            id INTEGER PRIMARY KEY,
            uuid TEXT,
            name TEXT NOT NULL,
            sort_name TEXT,
            mbid TEXT
        );

        CREATE TABLE IF NOT EXISTS albums (
            id INTEGER PRIMARY KEY,
            uuid TEXT,
            title TEXT NOT NULL,
            sort_title TEXT,
            artist_id INTEGER,
            year INTEGER,
            original_year INTEGER,
            edition_year INTEGER,
            edition_label TEXT,
            orphaned_at INTEGER,
            mbid TEXT,
            cover_art_path TEXT,
            caa_fail_count INTEGER,
            caa_last_error TEXT,
            caa_release_candidates TEXT,
            FOREIGN KEY(artist_id) REFERENCES artists(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS tracks (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            file_name TEXT NOT NULL,
            title TEXT,
            artist_id INTEGER,
            album_id INTEGER,
            track_number INTEGER,
            disc_number INTEGER,
            duration_ms INTEGER,
            sample_rate INTEGER,
            bit_depth INTEGER,
            format TEXT,
            mtime_ms INTEGER,
            size_bytes INTEGER,
            mbid TEXT,
            mb_no_match_key TEXT,
            FOREIGN KEY(artist_id) REFERENCES artists(id) ON DELETE SET NULL,
            FOREIGN KEY(album_id) REFERENCES albums(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS artist_bios (
            artist_id INTEGER NOT NULL,
            lang TEXT NOT NULL,
            text TEXT NOT NULL,
            source TEXT,
            locked INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER,
            PRIMARY KEY (artist_id, lang),
            FOREIGN KEY(artist_id) REFERENCES artists(id) ON DELETE CASCADE
        );


        CREATE TABLE IF NOT EXISTS album_notes (
            album_id INTEGER NOT NULL,
            lang TEXT NOT NULL,
            text TEXT NOT NULL,
            source TEXT,
            locked INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER,
            PRIMARY KEY (album_id, lang),
            FOREIGN KEY(album_id) REFERENCES albums(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS media_assets (
            id INTEGER PRIMARY KEY,
            owner_type TEXT NOT NULL,
            owner_id INTEGER NOT NULL,
            kind TEXT NOT NULL,
            local_path TEXT NOT NULL,
            checksum TEXT,
            source_url TEXT,
            updated_at_ms INTEGER
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_artists_name ON artists(name);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_albums_title_artist ON albums(title, artist_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks(album_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_artist_id ON tracks(artist_id);
        CREATE INDEX IF NOT EXISTS idx_albums_artist_id ON albums(artist_id);
        CREATE INDEX IF NOT EXISTS idx_media_assets_owner_kind ON media_assets(owner_type, owner_id, kind);
        "#,
    )
    .context("create metadata schema")?;

    let version_raw: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let version = version_raw
        .as_deref()
        .and_then(|value| value.parse::<i32>().ok());
    if version.is_none() {
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("insert schema version")?;
        ensure_uuid_indexes(conn)?;
        return Ok(());
    }
    let version = version.unwrap_or(1);
    if version < 2 {
        conn.execute("ALTER TABLE tracks ADD COLUMN mb_no_match_key TEXT", [])
            .context("migrate tracks mb_no_match_key")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
        return Ok(());
    }
    if version < 3 {
        conn.execute("ALTER TABLE albums ADD COLUMN caa_fail_count INTEGER", [])
            .context("migrate albums caa_fail_count")?;
        conn.execute("ALTER TABLE albums ADD COLUMN caa_last_error TEXT", [])
            .context("migrate albums caa_last_error")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
        return Ok(());
    }
    if version < 4 {
        conn.execute(
            "ALTER TABLE albums ADD COLUMN caa_release_candidates TEXT",
            [],
        )
        .context("migrate albums caa_release_candidates")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 5 {
        conn.execute("ALTER TABLE tracks ADD COLUMN bit_depth INTEGER", [])
            .context("migrate tracks bit_depth")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 6 {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS artist_bios (
                artist_id INTEGER NOT NULL,
                lang TEXT NOT NULL,
                text TEXT NOT NULL,
                source TEXT,
                locked INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER,
                PRIMARY KEY (artist_id, lang),
                FOREIGN KEY(artist_id) REFERENCES artists(id) ON DELETE CASCADE
            );


            CREATE TABLE IF NOT EXISTS album_notes (
                album_id INTEGER NOT NULL,
                lang TEXT NOT NULL,
                text TEXT NOT NULL,
                source TEXT,
                locked INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER,
                PRIMARY KEY (album_id, lang),
                FOREIGN KEY(album_id) REFERENCES albums(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS media_assets (
                id INTEGER PRIMARY KEY,
                owner_type TEXT NOT NULL,
                owner_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                local_path TEXT NOT NULL,
                checksum TEXT,
                source_url TEXT,
                updated_at_ms INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_media_assets_owner_kind ON media_assets(owner_type, owner_id, kind);
            "#,
        )
        .context("migrate bio/notes/assets tables")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 7 {
        conn.execute_batch(
            r#"
            DROP TABLE IF EXISTS artist_histories;
            DROP TABLE IF EXISTS album_histories;
            "#,
        )
        .context("drop history tables")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 8 {
        conn.execute_batch(
            r#"
            ALTER TABLE artists ADD COLUMN uuid TEXT;
            ALTER TABLE albums ADD COLUMN uuid TEXT;
            "#,
        )
        .context("add artist/album uuid columns")?;
        ensure_uuid_indexes(conn)?;
        backfill_uuids(conn, "artists")?;
        backfill_uuids(conn, "albums")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 9 {
        conn.execute_batch(
            r#"
            ALTER TABLE albums ADD COLUMN original_year INTEGER;
            ALTER TABLE albums ADD COLUMN edition_year INTEGER;
            ALTER TABLE albums ADD COLUMN edition_label TEXT;
            "#,
        )
        .context("add album edition columns")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    if version < 10 {
        conn.execute("ALTER TABLE albums ADD COLUMN orphaned_at INTEGER", [])
            .context("add album orphaned_at")?;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )
        .context("update schema version")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn db_path_for_places_db_under_audio_hub_dir() {
        let root = Path::new("/music/library");
        let path = db_path_for(root);
        assert_eq!(path, root.join(".audio-hub").join("metadata.sqlite"));
    }

    #[test]
    fn is_blank_handles_empty_and_whitespace() {
        assert!(is_blank(&None));
        assert!(is_blank(&Some("".to_string())));
        assert!(is_blank(&Some("  ".to_string())));
        assert!(!is_blank(&Some("a".to_string())));
    }

    #[test]
    fn find_artist_id_and_album_id_work() {
        let conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE artists (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE albums (id INTEGER PRIMARY KEY, title TEXT NOT NULL, artist_id INTEGER);
            "#,
        )
        .expect("create schema");
        conn.execute("INSERT INTO artists (id, name) VALUES (1, 'A-Ha')", [])
            .expect("insert artist");
        conn.execute(
            "INSERT INTO albums (id, title, artist_id) VALUES (10, 'Hunting High and Low', 1)",
            [],
        )
        .expect("insert album");

        let artist_id = find_artist_id(&conn, "A-Ha").expect("find artist");
        assert_eq!(artist_id, Some(1));
        let album_id = find_album_id(&conn, "Hunting High and Low", Some(1)).expect("find album");
        assert_eq!(album_id, Some(10));

        let missing_artist = find_artist_id(&conn, "Missing").expect("find missing artist");
        assert_eq!(missing_artist, None);
        let missing_album = find_album_id(&conn, "Missing", Some(1)).expect("find missing album");
        assert_eq!(missing_album, None);
    }

    #[test]
    fn relative_from_absolute_supports_root_basename_heuristic() {
        let tmp = std::env::temp_dir().join(format!(
            "audio-hub-path-rel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = tmp.join("music");
        let track = root.join("Artist").join("Album").join("song.flac");
        fs::create_dir_all(track.parent().unwrap()).expect("create track dir");
        fs::write(&track, b"audio").expect("write track file");

        let legacy = Path::new("/legacy/mount/music/Artist/Album/song.flac");
        let rel = relative_from_absolute(legacy, &root).expect("relative path");
        assert_eq!(rel, PathBuf::from("Artist/Album/song.flac"));
    }
}

/// Insert-or-fetch artist id by name and ensure UUID presence.
fn upsert_artist(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO artists (uuid, name, sort_name) VALUES (?1, ?2, ?3)",
        params![Uuid::new_v4().to_string(), name, name.to_lowercase()],
    )
    .context("upsert artist")?;
    let id: i64 = conn.query_row(
        "SELECT id FROM artists WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    ensure_row_uuid(conn, "artists", id)?;
    Ok(id)
}

/// Insert-or-fetch album id by `(title, artist_id)` and ensure UUID presence.
fn upsert_album(
    conn: &Connection,
    title: &str,
    artist_id: Option<i64>,
    year: Option<i32>,
) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO albums (uuid, title, artist_id, year, sort_title) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![Uuid::new_v4().to_string(), title, artist_id, year, title.to_lowercase()],
    )
    .context("upsert album")?;
    let id: i64 = conn.query_row(
        "SELECT id FROM albums WHERE title = ?1 AND artist_id IS ?2",
        params![title, artist_id],
        |row| row.get(0),
    )?;
    ensure_row_uuid(conn, "albums", id)?;
    Ok(id)
}

/// Insert or update album row keyed by explicit album UUID.
fn upsert_album_with_uuid(
    conn: &Connection,
    uuid: &str,
    title: &str,
    artist_id: Option<i64>,
    year: Option<i32>,
) -> Result<i64> {
    let existing: Option<(i64, Option<String>, Option<i64>, Option<i32>)> = conn
        .query_row(
            "SELECT id, title, artist_id, year FROM albums WHERE uuid = ?1",
            params![uuid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .context("lookup album by uuid")?;
    let id = if let Some((id, current_title, current_artist_id, current_year)) = existing {
        let mut needs_update = false;
        if current_title.as_deref() != Some(title) {
            needs_update = true;
        }
        if artist_id.is_some() && current_artist_id != artist_id {
            needs_update = true;
        }
        if year.is_some() && current_year != year {
            needs_update = true;
        }
        if needs_update {
            conn.execute(
                "UPDATE albums SET title = ?1, artist_id = COALESCE(?2, artist_id), year = COALESCE(?3, year), sort_title = ?4 WHERE id = ?5",
                params![title, artist_id, year, title.to_lowercase(), id],
            )
            .context("update album by uuid")?;
        }
        id
    } else {
        conn.execute(
            "INSERT INTO albums (uuid, title, artist_id, year, sort_title) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid, title, artist_id, year, title.to_lowercase()],
        )
        .context("insert album with uuid")?;
        conn.query_row(
            "SELECT id FROM albums WHERE uuid = ?1",
            params![uuid],
            |row| row.get(0),
        )?
    };
    Ok(id)
}
