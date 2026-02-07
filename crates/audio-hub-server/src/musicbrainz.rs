//! MusicBrainz lookup client for metadata enrichment.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::MusicBrainzConfig;
use crate::events::{EventBus, MetadataEvent};
use crate::state::MetadataWake;
use crate::metadata_db::{MetadataDb, MusicBrainzCandidate, TrackRecord};

const DEFAULT_BASE_URL: &str = "https://musicbrainz.org/ws/2";
const DEFAULT_RATE_LIMIT_MS: u64 = 1000;
const MIN_MATCH_SCORE: i32 = 90;

#[derive(Debug, Clone)]
pub struct MusicBrainzMatch {
    pub recording_mbid: Option<String>,
    pub artist_mbid: Option<String>,
    pub artist_name: Option<String>,
    pub artist_sort_name: Option<String>,
    pub album_mbid: Option<String>,
    pub album_title: Option<String>,
    pub release_year: Option<i32>,
    pub release_candidates: Vec<String>,
}

pub struct MusicBrainzClient {
    base_url: String,
    user_agent: String,
    rate_limit: Duration,
    last_request: Mutex<Instant>,
    agent: ureq::Agent,
}

impl MusicBrainzClient {
    pub fn new(cfg: &MusicBrainzConfig) -> Result<Option<Self>> {
        if !cfg.enabled.unwrap_or(false) {
            return Ok(None);
        }
        let Some(user_agent) = cfg.user_agent.as_ref() else {
            tracing::warn!("musicbrainz enabled but user_agent is missing");
            return Ok(None);
        };
        let base_url = cfg
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_string();
        let rate_limit = Duration::from_millis(cfg.rate_limit_ms.unwrap_or(DEFAULT_RATE_LIMIT_MS));
        let config = ureq::Agent::config_builder()
            .user_agent(user_agent)
            .build();
        let agent = ureq::Agent::new_with_config(config);

        Ok(Some(Self {
            base_url,
            user_agent: user_agent.to_string(),
            rate_limit,
            last_request: Mutex::new(Instant::now() - rate_limit),
            agent,
        }))
    }

    pub fn lookup_recording(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
    ) -> Result<MusicBrainzLookup> {
        let query = build_query(title, artist, album);
        let best = self.search_best_recording(&query)?;
        let Some(best) = best else {
            return Ok(MusicBrainzLookup::NoMatch {
                query,
                top_score: None,
                best_recording_id: None,
                best_recording_title: None,
            });
        };
        if best.score.unwrap_or(0) < MIN_MATCH_SCORE {
            return Ok(MusicBrainzLookup::NoMatch {
                query,
                top_score: best.score,
                best_recording_id: Some(best.id.clone()),
                best_recording_title: Some(best.title.clone()),
            });
        }

        let (artist_mbid, artist_name, artist_sort_name) = best
            .artist_credit
            .as_ref()
            .and_then(|credits| credits.first())
            .map(|credit| {
                (
                    Some(credit.artist.id.clone()),
                    Some(credit.artist.name.clone()),
                    credit.artist.sort_name.clone(),
                )
            })
            .unwrap_or((None, None, None));

        let (album_mbid, album_title, release_year, release_candidates) = best
            .releases
            .as_ref()
            .map(|releases| {
                let mut ids: Vec<String> = releases.iter().map(|r| r.id.clone()).collect();
                let mut seen = std::collections::HashSet::new();
                ids.retain(|id| seen.insert(id.clone()));
                let first = ids.first().cloned();
                let year = releases
                    .first()
                    .and_then(|release| release.date.as_deref())
                    .and_then(parse_year);
                let title = releases.first().map(|release| release.title.clone());
                let rest = ids.into_iter().skip(1).collect::<Vec<_>>();
                (first, title, year, rest)
            })
            .unwrap_or((None, None, None, Vec::new()));

        Ok(MusicBrainzLookup::Match(MusicBrainzMatch {
            recording_mbid: Some(best.id),
            artist_mbid,
            artist_name,
            artist_sort_name,
            album_mbid,
            album_title,
            release_year,
            release_candidates,
        }))
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    fn wait_rate_limit(&self) {
        let mut last = self
            .last_request
            .lock()
            .expect("musicbrainz rate limit lock");
        let elapsed = last.elapsed();
        if elapsed < self.rate_limit {
            std::thread::sleep(self.rate_limit - elapsed);
        }
        *last = Instant::now();
    }

    fn search_best_recording(&self, query: &str) -> Result<Option<RecordingResult>> {
        self.wait_rate_limit();

        let url = format!("{}/recording", self.base_url);
        let resp = self
            .agent
            .get(&url)
            .query("fmt", "json")
            .query("query", query)
            .call()
            .context("musicbrainz request failed")?;

        let body_str = resp
            .into_body()
            .with_config()
            .limit(1_000_000)
            .read_to_string()
            .context("musicbrainz response read failed")?;
        let body: RecordingSearchResponse = serde_json::from_str(&body_str)
            .context("musicbrainz response parse failed")?;

        Ok(body
            .recordings
            .into_iter()
            .max_by_key(|rec| rec.score.unwrap_or(0)))
    }
}

pub enum MusicBrainzLookup {
    Match(MusicBrainzMatch),
    NoMatch {
        query: String,
        top_score: Option<i32>,
        best_recording_id: Option<String>,
        best_recording_title: Option<String>,
    },
}

pub fn spawn_enrichment_loop(
    db: MetadataDb,
    client: std::sync::Arc<MusicBrainzClient>,
    events: EventBus,
    wake: MetadataWake,
) {
    std::thread::spawn(move || {
        let mut wake_seq = 0u64;
        loop {
            match db.list_musicbrainz_candidates(50) {
                Ok(candidates) => {
                    if !candidates.is_empty() {
                        tracing::info!(
                            count = candidates.len(),
                            "musicbrainz candidates fetched"
                        );
                        events.metadata_event(MetadataEvent::MusicBrainzBatch {
                            count: candidates.len(),
                        });
                    }
                    if candidates.is_empty() {
                        wake.wait(&mut wake_seq);
                        continue;
                    }
                    let mut attempted = 0usize;
                    for candidate in candidates {
                        match enrich_candidate(&db, &client, &events, &candidate) {
                            Ok(true) => attempted += 1,
                            Ok(false) => {}
                            Err(err) => {
                            tracing::warn!(
                                error = %err,
                                path = %candidate.path,
                                "musicbrainz background enrichment failed"
                            );
                            }
                        }
                    }
                    if attempted == 0 {
                        wake.wait(&mut wake_seq);
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "musicbrainz candidate query failed");
                    std::thread::sleep(Duration::from_secs(10));
                }
            }
        }
    });
}

fn enrich_candidate(
    db: &MetadataDb,
    client: &MusicBrainzClient,
    events: &EventBus,
    candidate: &MusicBrainzCandidate,
) -> Result<bool> {
    let key = no_match_key(&candidate.title, &candidate.artist, candidate.album.as_deref());
    if candidate.no_match_key.as_deref() == Some(key.as_str()) {
        return Ok(false);
    }
    events.metadata_event(MetadataEvent::MusicBrainzLookupStart {
        path: candidate.path.clone(),
        title: candidate.title.clone(),
        artist: candidate.artist.clone(),
        album: candidate.album.clone(),
    });
    let record = TrackRecord {
        path: candidate.path.clone(),
        file_name: String::new(),
        title: Some(candidate.title.clone()),
        artist: Some(candidate.artist.clone()),
        album: candidate.album.clone(),
        track_number: None,
        disc_number: None,
        year: None,
        duration_ms: None,
        sample_rate: None,
        format: None,
        mtime_ms: 0,
        size_bytes: 0,
    };
    let result = client.lookup_recording(
        candidate.title.as_str(),
        candidate.artist.as_str(),
        candidate.album.as_deref(),
    );
    match result {
        Ok(MusicBrainzLookup::Match(mb)) => {
            db.apply_musicbrainz(&record, &mb)?;
            events.metadata_event(MetadataEvent::MusicBrainzLookupSuccess {
                path: candidate.path.clone(),
                recording_mbid: mb.recording_mbid.clone(),
                artist_mbid: mb.artist_mbid.clone(),
                album_mbid: mb.album_mbid.clone(),
            });
        }
        Ok(MusicBrainzLookup::NoMatch {
            query,
            top_score,
            best_recording_id,
            best_recording_title,
        }) => {
            let _ = db.set_musicbrainz_no_match(&candidate.path, &key);
            events.metadata_event(MetadataEvent::MusicBrainzLookupNoMatch {
                path: candidate.path.clone(),
                title: candidate.title.clone(),
                artist: candidate.artist.clone(),
                album: candidate.album.clone(),
                query,
                top_score,
                best_recording_id,
                best_recording_title,
            });
        }
        Err(err) => {
            events.metadata_event(MetadataEvent::MusicBrainzLookupFailure {
                path: candidate.path.clone(),
                error: err.to_string(),
            });
        }
    }
    Ok(true)
}

fn no_match_key(title: &str, artist: &str, album: Option<&str>) -> String {
    let mut key = String::new();
    key.push_str(title.trim().to_lowercase().as_str());
    key.push('|');
    key.push_str(artist.trim().to_lowercase().as_str());
    key.push('|');
    if let Some(album) = album {
        key.push_str(album.trim().to_lowercase().as_str());
    }
    key
}

fn build_query(title: &str, artist: &str, album: Option<&str>) -> String {
    let mut parts = Vec::new();
    parts.push(format!("recording:\"{}\"", escape_query(title)));
    parts.push(format!("artist:\"{}\"", escape_query(artist)));
    if let Some(album) = album {
        parts.push(format!("release:\"{}\"", escape_query(album)));
    }
    parts.join(" AND ")
}

fn escape_query(raw: &str) -> String {
    raw.replace('"', "\\\"")
}

fn parse_year(raw: &str) -> Option<i32> {
    raw.split('-').next()?.trim().parse::<i32>().ok()
}

#[derive(Debug, Deserialize)]
struct RecordingSearchResponse {
    recordings: Vec<RecordingResult>,
}

#[derive(Debug, Deserialize)]
struct RecordingResult {
    id: String,
    score: Option<i32>,
    title: String,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<ArtistCredit>>,
    releases: Option<Vec<ReleaseSummary>>,
}

#[derive(Debug, Deserialize)]
struct ArtistCredit {
    artist: ArtistSummary,
}

#[derive(Debug, Deserialize)]
struct ArtistSummary {
    id: String,
    name: String,
    #[serde(rename = "sort-name")]
    sort_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseSummary {
    id: String,
    title: String,
    date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_query_includes_release_when_present() {
        let query = build_query("Track", "Artist", Some("Album"));
        assert_eq!(
            query,
            "recording:\"Track\" AND artist:\"Artist\" AND release:\"Album\""
        );
    }

    #[test]
    fn build_query_escapes_quotes() {
        let query = build_query("Track \"One\"", "Artist", None);
        assert_eq!(query, "recording:\"Track \\\"One\\\"\" AND artist:\"Artist\"");
    }

    #[test]
    fn parse_year_handles_full_date() {
        assert_eq!(parse_year("1999-04-01"), Some(1999));
    }
}
