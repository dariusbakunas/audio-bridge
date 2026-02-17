//! MusicBrainz lookup client for metadata enrichment.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
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
    pub artist_sort_name: Option<String>,
    pub album_mbid: Option<String>,
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

#[derive(Debug, Clone)]
pub struct MusicBrainzRecordingCandidate {
    pub recording_mbid: String,
    pub score: Option<i32>,
    pub title: String,
    pub artist_name: Option<String>,
    pub artist_mbid: Option<String>,
    pub release_title: Option<String>,
    pub release_mbid: Option<String>,
    pub year: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct MusicBrainzReleaseCandidate {
    pub release_mbid: String,
    pub score: Option<i32>,
    pub title: String,
    pub artist_name: Option<String>,
    pub artist_mbid: Option<String>,
    pub year: Option<i32>,
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
        let mut best = self.search_best_recording(&query)?;
        let mut query_label = query.clone();

        if best.as_ref().map(|rec| rec.score.unwrap_or(0) < MIN_MATCH_SCORE).unwrap_or(true) {
            if let Some((fallback_title, fallback_album)) = fallback_parts(title, album) {
                let fallback_query = build_query(&fallback_title, artist, fallback_album.as_deref());
                if fallback_query != query {
                    let fallback_best = self.search_best_recording(&fallback_query)?;
                    let fallback_score = fallback_best.as_ref().map(|rec| rec.score.unwrap_or(0));
                    if let Some(score) = fallback_score {
                        if score >= MIN_MATCH_SCORE {
                            best = fallback_best;
                            query_label = fallback_query;
                        } else {
                            best = select_best_recording(best, fallback_best);
                            query_label = format!("{query} | fallback: {fallback_query}");
                        }
                    } else if best.is_none() {
                        query_label = format!("{query} | fallback: {fallback_query}");
                    }
                }
            }
        }

        let Some(best) = best else {
            return Ok(MusicBrainzLookup::NoMatch {
                query: query_label,
                top_score: None,
                best_recording_id: None,
                best_recording_title: None,
            });
        };
        if best.score.unwrap_or(0) < MIN_MATCH_SCORE {
            return Ok(MusicBrainzLookup::NoMatch {
                query: query_label,
                top_score: best.score,
                best_recording_id: Some(best.id.clone()),
                best_recording_title: Some(best.title.clone()),
            });
        }

        let (artist_mbid, _artist_name, artist_sort_name) = best
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

        let (mut album_mbid, _album_title, mut release_year, mut release_candidates) = best
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

        if album_mbid.is_none() {
            if let Some(album_name) = album.map(str::trim).filter(|value| !value.is_empty()) {
                if let Ok(releases) = self.search_releases(album_name, artist, 5) {
                    if let Some(first) = releases.first() {
                        album_mbid = Some(first.release_mbid.clone());
                        if release_year.is_none() {
                            release_year = first.year;
                        }
                        release_candidates = releases
                            .into_iter()
                            .skip(1)
                            .map(|release| release.release_mbid)
                            .collect();
                    }
                }
            }
        }

        Ok(MusicBrainzLookup::Match(MusicBrainzMatch {
            recording_mbid: Some(best.id),
            artist_mbid,
            artist_sort_name,
            album_mbid,
            release_year,
            release_candidates,
        }))
    }

    pub fn search_recordings(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MusicBrainzRecordingCandidate>> {
        let query = build_query(title, artist, album);
        self.wait_rate_limit();

        let url = format!("{}/recording", self.base_url);
        let resp = self.call_request(
            self.agent
                .get(&url)
                .query("fmt", "json")
                .query("query", &query)
                .query("limit", &limit.to_string()),
            &url,
        )?;

        let body_str = resp
            .into_body()
            .with_config()
            .limit(1_000_000)
            .read_to_string()
            .context("musicbrainz response read failed")?;
        let body: RecordingSearchResponse = serde_json::from_str(&body_str)
            .context("musicbrainz response parse failed")?;

        let mut results = body
            .recordings
            .into_iter()
            .map(|rec| {
                let (artist_mbid, artist_name) = primary_artist(rec.artist_credit.as_ref());
                let (release_title, release_mbid, year) = rec
                    .releases
                    .as_ref()
                    .and_then(|releases| releases.first())
                    .map(|release| {
                        (
                            Some(release.title.clone()),
                            Some(release.id.clone()),
                            release
                                .date
                                .as_deref()
                                .and_then(parse_year),
                        )
                    })
                    .unwrap_or((None, None, None));
                MusicBrainzRecordingCandidate {
                    recording_mbid: rec.id,
                    score: rec.score,
                    title: rec.title,
                    artist_name,
                    artist_mbid,
                    release_title,
                    release_mbid,
                    year,
                }
            })
            .collect::<Vec<_>>();

        results.sort_by(|a, b| b.score.unwrap_or(0).cmp(&a.score.unwrap_or(0)));
        if results.is_empty() {
            if let Some((fallback_title, fallback_album)) = fallback_parts(title, album) {
                let fallback_query =
                    build_query(&fallback_title, artist, fallback_album.as_deref());
                if fallback_query != query {
                    return self.search_recordings(
                        &fallback_title,
                        artist,
                        fallback_album.as_deref(),
                        limit,
                    );
                }
            }
        }
        Ok(results)
    }

    pub fn search_releases(
        &self,
        title: &str,
        artist: &str,
        limit: u32,
    ) -> Result<Vec<MusicBrainzReleaseCandidate>> {
        let query = build_release_query(title, artist);
        self.wait_rate_limit();

        let url = format!("{}/release", self.base_url);
        let resp = self.call_request(
            self.agent
                .get(&url)
                .query("fmt", "json")
                .query("query", &query)
                .query("limit", &limit.to_string()),
            &url,
        )?;

        let body_str = resp
            .into_body()
            .with_config()
            .limit(1_000_000)
            .read_to_string()
            .context("musicbrainz response read failed")?;
        let body: ReleaseSearchResponse = serde_json::from_str(&body_str)
            .context("musicbrainz response parse failed")?;

        let mut results = body
            .releases
            .into_iter()
            .map(|release| {
                let (artist_mbid, artist_name) = primary_artist(release.artist_credit.as_ref());
                MusicBrainzReleaseCandidate {
                    release_mbid: release.id,
                    score: release.score,
                    title: release.title,
                    artist_name,
                    artist_mbid,
                    year: release
                        .date
                        .as_deref()
                        .and_then(parse_year),
                }
            })
            .collect::<Vec<_>>();

        results.sort_by(|a, b| b.score.unwrap_or(0).cmp(&a.score.unwrap_or(0)));
        if results.is_empty() {
            if let Some(fallback_title) = strip_parenthetical(title) {
                let fallback_query = build_release_query(&fallback_title, artist);
                if fallback_query != query {
                    return self.search_releases(&fallback_title, artist, limit);
                }
            }
        }
        Ok(results)
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

    fn call_request(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
        url: &str,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let resp = match request.config().http_status_as_error(false).build().call() {
            Ok(resp) => resp,
            Err(err) => {
                bail!("musicbrainz request failed (transport) url={url}: {err}");
            }
        };
        let code = resp.status();
        if code.as_u16() >= 400 {
            let body = resp
                .into_body()
                .with_config()
                .limit(200_000)
                .read_to_string()
                .unwrap_or_default();
            let trimmed = body.trim();
            if trimmed.is_empty() {
                bail!("musicbrainz request failed (status {code}) url={url}");
            }
            let snippet: String = trimmed.chars().take(300).collect();
            let suffix = if trimmed.chars().count() > 300 { "..." } else { "" };
            bail!("musicbrainz request failed (status {code}) url={url}: {snippet}{suffix}");
        }
        Ok(resp)
    }

    fn search_best_recording(&self, query: &str) -> Result<Option<RecordingResult>> {
        self.wait_rate_limit();

        let url = format!("{}/recording", self.base_url);
        let resp = self.call_request(
            self.agent
                .get(&url)
                .query("fmt", "json")
                .query("query", query),
            &url,
        )?;

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
        album_artist: candidate.album_artist.clone(),
        album: candidate.album.clone(),
        album_uuid: None,
        track_number: None,
        disc_number: None,
        year: None,
        duration_ms: None,
        sample_rate: None,
        bit_depth: None,
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

fn build_release_query(title: &str, artist: &str) -> String {
    format!(
        "release:\"{}\" AND artist:\"{}\"",
        escape_query(title),
        escape_query(artist)
    )
}

fn escape_query(raw: &str) -> String {
    raw.replace('"', "\\\"")
}

fn strip_parenthetical(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut depth = 0usize;
    let mut prev_space = false;
    for ch in raw.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth = depth.saturating_add(1);
            }
            ')' | ']' | '}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    if ch.is_whitespace() {
                        if !prev_space {
                            out.push(' ');
                            prev_space = true;
                        }
                    } else {
                        out.push(ch);
                        prev_space = false;
                    }
                }
            }
        }
    }
    let cleaned = out.trim();
    if cleaned.is_empty() {
        return None;
    }
    let without_suffix = strip_suffix_after_dash(cleaned);
    if without_suffix.eq_ignore_ascii_case(raw.trim()) {
        None
    } else {
        Some(without_suffix)
    }
}

fn strip_suffix_after_dash(value: &str) -> String {
    if let Some((left, _)) = value.split_once(" - ") {
        let trimmed = left.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    value.trim().to_string()
}

fn fallback_parts(title: &str, album: Option<&str>) -> Option<(String, Option<String>)> {
    let title_fallback = strip_parenthetical(title);
    let album_fallback = album.and_then(strip_parenthetical);
    if title_fallback.is_none() && album_fallback.is_none() {
        return None;
    }
    let title = title_fallback.unwrap_or_else(|| title.trim().to_string());
    let album = album
        .map(|value| {
            album_fallback
                .unwrap_or_else(|| value.trim().to_string())
        })
        .filter(|value| !value.is_empty());
    Some((title, album))
}

fn select_best_recording(
    left: Option<RecordingResult>,
    right: Option<RecordingResult>,
) -> Option<RecordingResult> {
    match (left, right) {
        (Some(a), Some(b)) => {
            if a.score.unwrap_or(0) >= b.score.unwrap_or(0) {
                Some(a)
            } else {
                Some(b)
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
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
struct ReleaseSearchResponse {
    releases: Vec<ReleaseResult>,
}

#[derive(Debug, Deserialize)]
struct ReleaseResult {
    id: String,
    score: Option<i32>,
    title: String,
    date: Option<String>,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<ArtistCredit>>,
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

fn primary_artist(
    credits: Option<&Vec<ArtistCredit>>,
) -> (Option<String>, Option<String>) {
    credits
        .and_then(|items| items.first())
        .map(|credit| {
            (
                Some(credit.artist.id.clone()),
                Some(credit.artist.name.clone()),
            )
        })
        .unwrap_or((None, None))
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
    fn build_release_query_escapes_quotes() {
        let query = build_release_query("Album \"One\"", "Artist");
        assert_eq!(query, "release:\"Album \\\"One\\\"\" AND artist:\"Artist\"");
    }

    #[test]
    fn parse_year_handles_full_date() {
        assert_eq!(parse_year("1999-04-01"), Some(1999));
    }

    #[test]
    fn strip_parenthetical_removes_suffix() {
        let stripped = strip_parenthetical("Hunting High and Low (2015 Remaster)");
        assert_eq!(stripped.as_deref(), Some("Hunting High and Low"));
    }

    #[test]
    fn strip_parenthetical_removes_dash_suffix() {
        let stripped = strip_parenthetical("One Assassination Under God - Chapter 1");
        assert_eq!(stripped.as_deref(), Some("One Assassination Under God"));
    }

    #[test]
    fn strip_parenthetical_removes_brackets() {
        let stripped = strip_parenthetical("Album Title [Deluxe Edition]");
        assert_eq!(stripped.as_deref(), Some("Album Title"));
    }

    #[test]
    fn fallback_parts_returns_none_when_no_change() {
        let fallback = fallback_parts("Album", Some("Album"));
        assert!(fallback.is_none());
    }

    #[test]
    fn fallback_parts_strips_album_only() {
        let fallback = fallback_parts("Track", Some("Album (Deluxe)"));
        assert_eq!(fallback, Some(("Track".to_string(), Some("Album".to_string()))));
    }

    #[test]
    fn select_best_recording_prefers_higher_score() {
        let low = RecordingResult {
            id: "low".to_string(),
            score: Some(10),
            title: "Low".to_string(),
            artist_credit: None,
            releases: None,
        };
        let high = RecordingResult {
            id: "high".to_string(),
            score: Some(95),
            title: "High".to_string(),
            artist_credit: None,
            releases: None,
        };
        let best = select_best_recording(Some(low), Some(high)).unwrap();
        assert_eq!(best.id, "high");
    }

    #[test]
    fn select_best_recording_handles_none() {
        let single = RecordingResult {
            id: "one".to_string(),
            score: None,
            title: "Single".to_string(),
            artist_credit: None,
            releases: None,
        };
        let best = select_best_recording(None, Some(single)).unwrap();
        assert_eq!(best.id, "one");
    }

    #[test]
    #[ignore]
    fn live_lookup_release_and_cover_art() {
        let cfg = MusicBrainzConfig {
            enabled: Some(true),
            user_agent: Some("audio-hub-tests/0.1 (local testing)".to_string()),
            base_url: None,
            rate_limit_ms: Some(1000),
        };
        let client = MusicBrainzClient::new(&cfg)
            .expect("client init")
            .expect("client enabled");
        let releases = client
            .search_releases("The Getaway", "Red Hot Chili Peppers", 5)
            .expect("search releases");
        assert!(!releases.is_empty(), "expected release candidates");
        let mbid = releases
            .iter()
            .find(|rel| rel.score.unwrap_or(0) > 0)
            .map(|rel| rel.release_mbid.as_str())
            .unwrap_or_else(|| releases[0].release_mbid.as_str());
        let (mime, bytes) = crate::cover_art::fetch_cover_front(mbid, client.user_agent())
            .expect("cover art fetch");
        assert!(
            mime.starts_with("image/"),
            "expected image mime, got {mime}"
        );
        assert!(bytes.len() > 1024, "expected non-empty cover art");
    }

    #[test]
    #[ignore]
    fn live_release_search_with_parenthetical() {
        let cfg = MusicBrainzConfig {
            enabled: Some(true),
            user_agent: Some("audio-hub-tests/0.1 (local testing)".to_string()),
            base_url: None,
            rate_limit_ms: Some(1000),
        };
        let client = MusicBrainzClient::new(&cfg)
            .expect("client init")
            .expect("client enabled");
        let releases = client
            .search_releases("Hunting High and Low (2015 Remaster)", "A-ha", 5)
            .expect("search releases");
        assert!(
            releases.iter().any(|rel| rel.title.contains("Hunting High and Low")),
            "expected a matching release title"
        );
    }
}
