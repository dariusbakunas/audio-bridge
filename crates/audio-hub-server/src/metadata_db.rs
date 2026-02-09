//! SQLite metadata store for artists/albums/tracks.
//!
//! Provides pooled connections and schema bootstrap.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use crate::musicbrainz::MusicBrainzMatch;
const SCHEMA_VERSION: i32 = 4;

#[derive(Clone)]
pub struct MetadataDb {
    pool: Pool<SqliteConnectionManager>,
}

#[derive(Debug, Clone)]
pub struct TrackRecord {
    pub path: String,
    pub file_name: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub format: Option<String>,
    pub mtime_ms: i64,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ArtistSummary {
    pub id: i64,
    pub name: String,
    pub sort_name: Option<String>,
    pub mbid: Option<String>,
    pub album_count: i64,
    pub track_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AlbumSummary {
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub mbid: Option<String>,
    pub track_count: i64,
    pub cover_art_path: Option<String>,
    pub cover_art_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TrackSummary {
    pub id: i64,
    pub path: String,
    pub file_name: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub format: Option<String>,
    pub mbid: Option<String>,
    pub cover_art_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MusicBrainzCandidate {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub no_match_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CoverArtCandidate {
    pub album_id: i64,
    pub mbid: String,
    pub fail_count: i64,
    pub release_candidates: Vec<String>,
}

fn map_artist_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtistSummary> {
    Ok(ArtistSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_name: row.get(2)?,
        mbid: row.get(3)?,
        album_count: row.get(4)?,
        track_count: row.get(5)?,
    })
}

impl MetadataDb {
    pub fn new(media_root: &Path) -> Result<Self> {
        let db_path = db_path_for(media_root);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create metadata dir {:?}", parent))?;
        }

        let manager = SqliteConnectionManager::file(&db_path)
            .with_init(|conn| {
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

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool<SqliteConnectionManager> {
        &self.pool
    }

    pub fn upsert_track(&self, record: &TrackRecord) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;

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
                       t.disc_number
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                LEFT JOIN artists aa ON aa.id = al.artist_id
                WHERE t.path = ?1
                "#,
                params![record.path],
                |row| Ok((
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
                )),
            )
            .optional()
            .context("lookup existing track")?;
        let (existing_artist_id, existing_album_id, keep_album_link) = if let Some((
            mtime_ms,
            size_bytes,
            artist_id,
            album_id,
            track_mbid,
            artist_mbid,
            album_mbid,
            album_title,
            album_artist,
            disc_number,
        )) = existing
        {
            let album_title_same = match (record.album.as_deref(), album_title.as_deref()) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            let desired_album_artist = record
                .album_artist
                .as_deref()
                .or(record.artist.as_deref());
            let album_artist_same = match (desired_album_artist, album_artist.as_deref()) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            let disc_same = record.disc_number == disc_number;
            if mtime_ms == record.mtime_ms
                && size_bytes == record.size_bytes
                && album_title_same
                && album_artist_same
                && disc_same
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
        let album_artist_id = if let Some(name) = record
            .album_artist
            .as_deref()
            .or(record.artist.as_deref())
        {
            Some(upsert_artist(&tx, name)?)
        } else {
            None
        };
        let album_id = if keep_album_link {
            existing_album_id
        } else if let Some(title) = record.album.as_deref() {
            Some(upsert_album(&tx, title, album_artist_id, record.year)?)
        } else {
            None
        };

        tx.execute(
            r#"
            INSERT INTO tracks (
                path, file_name, title, artist_id, album_id, track_number, disc_number,
                duration_ms, sample_rate, format, mtime_ms, size_bytes, mb_no_match_key
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(path) DO UPDATE SET
                file_name = excluded.file_name,
                title = excluded.title,
                artist_id = excluded.artist_id,
                album_id = excluded.album_id,
                track_number = excluded.track_number,
                disc_number = excluded.disc_number,
                duration_ms = excluded.duration_ms,
                sample_rate = excluded.sample_rate,
                format = excluded.format,
                mtime_ms = excluded.mtime_ms,
                size_bytes = excluded.size_bytes,
                mb_no_match_key = NULL
            "#,
            params![
                record.path,
                record.file_name,
                record.title,
                artist_id,
                album_id,
                record.track_number,
                record.disc_number,
                record.duration_ms.map(|v| v as i64),
                record.sample_rate.map(|v| v as i64),
                record.format,
                record.mtime_ms,
                record.size_bytes,
                Option::<String>::None
            ],
        )
        .context("upsert track")?;

        tx.commit().context("commit metadata tx")?;
        Ok(())
    }

    pub fn needs_musicbrainz(&self, path: &str) -> Result<bool> {
        let conn = self.pool.get().context("open metadata db")?;
        let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
            .query_row(
                r#"
                SELECT t.mbid, ar.mbid, al.mbid
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                WHERE t.path = ?1
                "#,
                params![path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .context("check musicbrainz metadata")?;
        let Some((track_mbid, artist_mbid, album_mbid)) = row else {
            return Ok(true);
        };
        Ok(is_blank(&track_mbid) || is_blank(&artist_mbid) || is_blank(&album_mbid))
    }

    pub fn apply_musicbrainz(
        &self,
        record: &TrackRecord,
        mb: &MusicBrainzMatch,
    ) -> Result<()> {
        self.apply_musicbrainz_with_override(record, mb, false)
    }

    pub fn apply_musicbrainz_with_override(
        &self,
        record: &TrackRecord,
        mb: &MusicBrainzMatch,
        override_existing: bool,
    ) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;

        let artist_id = if let Some(name) = record.artist.as_deref() {
            find_artist_id(&tx, name)?
        } else {
            None
        };
        let album_id = tx
            .query_row(
                "SELECT album_id FROM tracks WHERE path = ?1",
                params![record.path],
                |row| row.get(0),
            )
            .optional()
            .context("fetch album id for track")?;
        let album_id = if album_id.is_some() {
            album_id
        } else if let Some(title) = record.album.as_deref() {
            let album_artist_id = if let Some(name) = record
                .album_artist
                .as_deref()
                .or(record.artist.as_deref())
            {
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
                    params![recording_mbid, record.path],
                )
                .context("update track mbid")?;
            } else {
                tx.execute(
                    "UPDATE tracks SET mbid = ?1, mb_no_match_key = NULL WHERE path = ?2 AND (mbid IS NULL OR mbid = '')",
                    params![recording_mbid, record.path],
                )
                .context("update track mbid")?;
            }
        }

        tx.commit().context("commit metadata tx")?;
        Ok(())
    }

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

    pub fn track_record_by_path(&self, path: &str) -> Result<Option<TrackRecord>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn
            .query_row(
                r#"
                SELECT t.path, t.file_name, t.title, ar.name, aa.name, al.title,
                       t.track_number, t.disc_number, al.year, t.duration_ms,
                       t.sample_rate, t.format, t.mtime_ms, t.size_bytes
                FROM tracks t
                LEFT JOIN artists ar ON ar.id = t.artist_id
                LEFT JOIN albums al ON al.id = t.album_id
                LEFT JOIN artists aa ON aa.id = al.artist_id
                WHERE t.path = ?1
                "#,
                params![path],
                |row| {
                    Ok(TrackRecord {
                        path: row.get(0)?,
                        file_name: row.get(1)?,
                        title: row.get(2)?,
                        artist: row.get(3)?,
                        album_artist: row.get(4)?,
                        album: row.get(5)?,
                        track_number: row
                            .get::<_, Option<i64>>(6)?
                            .map(|v| v as u32),
                        disc_number: row
                            .get::<_, Option<i64>>(7)?
                            .map(|v| v as u32),
                        year: row.get(8)?,
                        duration_ms: row
                            .get::<_, Option<i64>>(9)?
                            .map(|v| v as u64),
                        sample_rate: row
                            .get::<_, Option<i64>>(10)?
                            .map(|v| v as u32),
                        format: row.get(11)?,
                        mtime_ms: row.get(12)?,
                        size_bytes: row.get(13)?,
                    })
                },
            )
            .optional()
            .context("fetch track record")
    }

    pub fn album_id_for_track_path(&self, path: &str) -> Result<Option<i64>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn
            .query_row(
                "SELECT album_id FROM tracks WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()
            .context("fetch album id for track")
    }

    pub fn list_musicbrainz_candidates(
        &self,
        limit: i64,
    ) -> Result<Vec<MusicBrainzCandidate>> {
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
            Ok(MusicBrainzCandidate {
                path: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                album_artist: row.get(4)?,
                no_match_key: row.get(5)?,
            })
        })?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn album_cover_path(
        &self,
        album: &str,
        artist: Option<&str>,
    ) -> Result<Option<String>> {
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

    pub fn cover_path_for_track(&self, path: &str) -> Result<Option<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let cover: Option<String> = conn
            .query_row(
                r#"
                SELECT al.cover_art_path
                FROM tracks t
                LEFT JOIN albums al ON al.id = t.album_id
                WHERE t.path = ?1
                "#,
                params![path],
                |row| row.get(0),
            )
            .optional()
            .context("fetch cover path for track")?;
        Ok(cover.filter(|value| !value.trim().is_empty()))
    }

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

    pub fn set_album_cover_by_id_if_empty(
        &self,
        album_id: i64,
        cover_path: &str,
    ) -> Result<bool> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        let updated = tx.execute(
            "UPDATE albums SET cover_art_path = ?1, caa_fail_count = NULL, caa_last_error = NULL WHERE id = ?2 AND (cover_art_path IS NULL OR cover_art_path = '')",
            params![cover_path, album_id],
        )?;
        tx.commit().context("commit metadata tx")?;
        Ok(updated > 0)
    }

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
            let raw_candidates: Option<String> = row.get(3)?;
            let release_candidates = raw_candidates
                .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
                .unwrap_or_default();
            Ok(CoverArtCandidate {
                album_id: row.get(0)?,
                mbid: row.get(1)?,
                fail_count: row.get(2)?,
                release_candidates,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn increment_cover_art_fail(
        &self,
        album_id: i64,
        error: &str,
    ) -> Result<i64> {
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

    pub fn set_musicbrainz_no_match(&self, path: &str, key: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            "UPDATE tracks SET mb_no_match_key = ?1 WHERE path = ?2",
            params![key, path],
        )
        .context("set musicbrainz no match")?;
        Ok(())
    }

    pub fn clear_musicbrainz_no_match(&self, path: &str) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute(
            "UPDATE tracks SET mb_no_match_key = NULL WHERE path = ?1",
            params![path],
        )
        .context("clear musicbrainz no match")?;
        Ok(())
    }

    pub fn clear_musicbrainz_no_match_all(&self) -> Result<()> {
        let conn = self.pool.get().context("open metadata db")?;
        conn.execute("UPDATE tracks SET mb_no_match_key = NULL", [])
            .context("clear musicbrainz no match all")?;
        Ok(())
    }

    pub fn clear_library(&self) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata clear")?;
        tx.execute("DELETE FROM tracks", []).context("clear tracks")?;
        tx.execute("DELETE FROM albums", []).context("clear albums")?;
        tx.execute("DELETE FROM artists", []).context("clear artists")?;
        tx.execute("DELETE FROM sqlite_sequence", []).ok();
        tx.commit().context("commit metadata clear")?;
        Ok(())
    }

    pub fn list_artists(&self, search: Option<&str>, limit: i64, offset: i64) -> Result<Vec<ArtistSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        let search_like = search.map(|s| format!("%{}%", s.to_lowercase()));
        let mut stmt = if search_like.is_some() {
            conn.prepare(
                r#"
                SELECT a.id, a.name, a.sort_name, a.mbid,
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
                SELECT a.id, a.name, a.sort_name, a.mbid,
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
            SELECT al.id, al.title, ar.name, al.year, al.mbid,
                   COUNT(t.id) AS track_count, al.cover_art_path
            FROM albums al
            LEFT JOIN artists ar ON ar.id = al.artist_id
            LEFT JOIN tracks t ON t.album_id = al.id
            WHERE (?1 IS NULL OR al.artist_id = ?1)
              AND (?2 IS NULL OR LOWER(al.title) LIKE ?2)
            GROUP BY al.id
            ORDER BY
                CASE WHEN ar.name IS NULL THEN 1 ELSE 0 END,
                COALESCE(ar.sort_name, ar.name),
                COALESCE(al.sort_title, al.title)
            LIMIT ?3 OFFSET ?4
            "#,
        )?;
        let rows = stmt.query_map(
            params![artist_id, search_like, limit, offset],
            |row| {
                let album_id: i64 = row.get(0)?;
                let cover_path: Option<String> = row.get(6)?;
                let cover_art_url = cover_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| format!("/albums/{}/cover", album_id));
                Ok(AlbumSummary {
                    id: album_id,
                    title: row.get(1)?,
                    artist: row.get(2)?,
                    year: row.get(3)?,
                    mbid: row.get(4)?,
                    track_count: row.get(5)?,
                    cover_art_path: cover_path,
                    cover_art_url,
                })
            },
        )?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn album_summary_by_id(&self, album_id: i64) -> Result<Option<AlbumSummary>> {
        let conn = self.pool.get().context("open metadata db")?;
        conn
            .query_row(
                r#"
                SELECT al.id, al.title, ar.name, al.year, al.mbid,
                       COUNT(t.id) AS track_count, al.cover_art_path
                FROM albums al
                LEFT JOIN artists ar ON ar.id = al.artist_id
                LEFT JOIN tracks t ON t.album_id = al.id
                WHERE al.id = ?1
                GROUP BY al.id
                "#,
                params![album_id],
                |row| {
                    let album_id: i64 = row.get(0)?;
                    let cover_path: Option<String> = row.get(6)?;
                    let cover_art_url = cover_path
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .map(|_| format!("/albums/{}/cover", album_id));
                    Ok(AlbumSummary {
                        id: album_id,
                        title: row.get(1)?,
                        artist: row.get(2)?,
                        year: row.get(3)?,
                        mbid: row.get(4)?,
                        track_count: row.get(5)?,
                        cover_art_path: cover_path,
                        cover_art_url,
                    })
                },
            )
            .optional()
            .context("select album summary by id")
    }

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
            SELECT t.id, t.path, t.file_name, t.title, ar.name, al.title,
                   t.track_number, t.disc_number, t.duration_ms, t.format, t.mbid,
                   al.cover_art_path
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
                let cover_path: Option<String> = row.get(11)?;
                let cover_art_url = cover_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|_| format!("/tracks/{}/cover", track_id));
                Ok(TrackSummary {
                    id: track_id,
                    path: row.get(1)?,
                    file_name: row.get(2)?,
                    title: row.get(3)?,
                    artist: row.get(4)?,
                    album: row.get(5)?,
                    track_number: row.get::<_, Option<i64>>(6)?.map(|v| v as u32),
                    disc_number: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                    duration_ms: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    format: row.get(9)?,
                    mbid: row.get(10)?,
                    cover_art_url,
                })
            },
        )?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn list_track_paths_by_album_id(&self, album_id: i64) -> Result<Vec<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare(
            "SELECT path FROM tracks WHERE album_id = ?1 ORDER BY COALESCE(disc_number, 0), COALESCE(track_number, 0), file_name",
        )?;
        let rows = stmt.query_map(params![album_id], |row| row.get(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    pub fn list_all_track_paths(&self) -> Result<Vec<String>> {
        let conn = self.pool.get().context("open metadata db")?;
        let mut stmt = conn.prepare("SELECT path FROM tracks")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

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

    pub fn delete_track_by_path(&self, path: &str) -> Result<bool> {
        let conn = self.pool.get().context("open metadata db")?;
        let deleted = conn
            .execute("DELETE FROM tracks WHERE path = ?1", params![path])
            .context("delete track by path")?;
        Ok(deleted > 0)
    }

    pub fn prune_orphaned_albums_and_artists(&self) -> Result<()> {
        let mut conn = self.pool.get().context("open metadata db")?;
        let tx = conn.transaction().context("begin metadata tx")?;
        tx.execute(
            "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM tracks WHERE album_id IS NOT NULL)",
            [],
        )
        .context("delete orphaned albums")?;
        tx.execute(
            r#"
            DELETE FROM artists
            WHERE id NOT IN (
                SELECT DISTINCT artist_id FROM tracks WHERE artist_id IS NOT NULL
                UNION
                SELECT DISTINCT artist_id FROM albums WHERE artist_id IS NOT NULL
            )
            "#,
            [],
        )
        .context("delete orphaned artists")?;
        tx.commit().context("commit metadata tx")?;
        Ok(())
    }
}

fn db_path_for(media_root: &Path) -> PathBuf {
    media_root.join(".audio-hub").join("metadata.sqlite")
}

fn is_blank(value: &Option<String>) -> bool {
    value.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true)
}

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

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artists (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            sort_name TEXT,
            mbid TEXT
        );

        CREATE TABLE IF NOT EXISTS albums (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            sort_title TEXT,
            artist_id INTEGER,
            year INTEGER,
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
            format TEXT,
            mtime_ms INTEGER,
            size_bytes INTEGER,
            mbid TEXT,
            mb_no_match_key TEXT,
            FOREIGN KEY(artist_id) REFERENCES artists(id) ON DELETE SET NULL,
            FOREIGN KEY(album_id) REFERENCES albums(id) ON DELETE SET NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_artists_name ON artists(name);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_albums_title_artist ON albums(title, artist_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks(album_id);
        CREATE INDEX IF NOT EXISTS idx_tracks_artist_id ON tracks(artist_id);
        CREATE INDEX IF NOT EXISTS idx_albums_artist_id ON albums(artist_id);
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
        conn.execute("ALTER TABLE albums ADD COLUMN caa_release_candidates TEXT", [])
            .context("migrate albums caa_release_candidates")?;
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
}

fn upsert_artist(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO artists (name, sort_name) VALUES (?1, ?2)",
        params![name, name.to_lowercase()],
    )
    .context("upsert artist")?;
    let id: i64 = conn.query_row(
        "SELECT id FROM artists WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn upsert_album(conn: &Connection, title: &str, artist_id: Option<i64>, year: Option<i32>) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO albums (title, artist_id, year, sort_title) VALUES (?1, ?2, ?3, ?4)",
        params![title, artist_id, year, title.to_lowercase()],
    )
    .context("upsert album")?;
    let id: i64 = conn.query_row(
        "SELECT id FROM albums WHERE title = ?1 AND artist_id IS ?2",
        params![title, artist_id],
        |row| row.get(0),
    )?;
    Ok(id)
}
