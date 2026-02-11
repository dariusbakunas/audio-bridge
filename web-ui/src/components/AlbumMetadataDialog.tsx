import { useEffect, useMemo, useState } from "react";
import { fetchJson, postJson } from "../api";
import {
  AlbumMetadataResponse,
  AlbumMetadataUpdateResponse,
  MusicBrainzMatchCandidate,
  MusicBrainzMatchSearchResponse
} from "../types";
import Modal from "./Modal";

type AlbumDialogTab = "tags" | "match";

interface AlbumMetadataDialogProps {
  open: boolean;
  albumId: number | null;
  targetLabel: string;
  artist: string;
  defaults: {
    title?: string | null;
    albumArtist?: string | null;
    year?: number | null;
  };
  onBeforeUpdate?: () => Promise<void> | void;
  onClose: () => void;
  onUpdated?: (albumId: number) => void;
}

function parseOptionalInt(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number.parseInt(trimmed, 10);
  if (Number.isNaN(parsed) || parsed <= 0) return undefined;
  return parsed;
}

export default function AlbumMetadataDialog({
  open,
  albumId,
  targetLabel,
  artist,
  defaults,
  onBeforeUpdate,
  onClose,
  onUpdated
}: AlbumMetadataDialogProps) {
  const [activeTab, setActiveTab] = useState<AlbumDialogTab>("tags");
  const [title, setTitle] = useState(defaults.title ?? "");
  const [albumArtist, setAlbumArtist] = useState(defaults.albumArtist ?? "");
  const [year, setYear] = useState(defaults.year ? String(defaults.year) : "");
  const [initialTitle, setInitialTitle] = useState(defaults.title ?? "");
  const [initialAlbumArtist, setInitialAlbumArtist] = useState(defaults.albumArtist ?? "");
  const [initialYear, setInitialYear] = useState(defaults.year ? String(defaults.year) : "");
  const [trackArtist, setTrackArtist] = useState("");
  const [applyTrackArtist, setApplyTrackArtist] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [tagsError, setTagsError] = useState<string | null>(null);

  const [matchTitle, setMatchTitle] = useState(defaults.title ?? "");
  const [matchArtist, setMatchArtist] = useState(artist);
  const [results, setResults] = useState<MusicBrainzMatchCandidate[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  const [searching, setSearching] = useState(false);
  const [applying, setApplying] = useState(false);
  const [matchError, setMatchError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setActiveTab("tags");
    setTitle(defaults.title ?? "");
    setAlbumArtist(defaults.albumArtist ?? "");
    setYear(defaults.year ? String(defaults.year) : "");
    setInitialTitle(defaults.title ?? "");
    setInitialAlbumArtist(defaults.albumArtist ?? "");
    setInitialYear(defaults.year ? String(defaults.year) : "");
    setTrackArtist("");
    setApplyTrackArtist(false);
    setTagsError(null);
    setMatchTitle(defaults.title ?? "");
    setMatchArtist(artist);
    setResults([]);
    setSelectedIndex(null);
    setExpandedIndex(null);
    setMatchError(null);
  }, [
    open,
    defaults.title,
    defaults.albumArtist,
    defaults.year,
    artist
  ]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    if (!albumId) {
      setLoading(false);
      return;
    }
    setLoading(true);
    fetchJson<AlbumMetadataResponse>(`/albums/metadata?album_id=${albumId}`)
      .then((response) => {
        if (!active || !response) return;
        setTitle(response.title ?? "");
        setAlbumArtist(response.album_artist ?? "");
        setYear(response.year ? String(response.year) : "");
        setInitialTitle(response.title ?? "");
        setInitialAlbumArtist(response.album_artist ?? "");
        setInitialYear(response.year ? String(response.year) : "");
      })
      .catch((err) => {
        if (!active) return;
        setTagsError((err as Error).message);
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [open, albumId]);

  const selected = selectedIndex === null ? null : results[selectedIndex] ?? null;
  const canSearch = Boolean(matchTitle.trim() && matchArtist.trim());
  const hasRequiredIds = useMemo(() => {
    if (!selected || !albumId) return false;
    return Boolean(selected.release_mbid);
  }, [selected, albumId]);
  const titleValue = title.trim();
  const albumArtistValue = albumArtist.trim();
  const yearValue = year.trim();
  const trackArtistValue = trackArtist.trim();
  const yearParsed = yearValue ? parseOptionalInt(yearValue) : undefined;
  const isYearValid = !yearValue || yearParsed !== undefined;
  const isTrackArtistValid = !applyTrackArtist || Boolean(trackArtistValue);
  const hasTagChanges = useMemo(() => {
    const initialTitleValue = initialTitle.trim();
    const initialAlbumArtistValue = initialAlbumArtist.trim();
    const initialYearValue = initialYear.trim();
    const titleChanged = Boolean(titleValue) && titleValue !== initialTitleValue;
    const albumArtistChanged =
      Boolean(albumArtistValue) && albumArtistValue !== initialAlbumArtistValue;
    const yearChanged = Boolean(yearValue) && yearValue !== initialYearValue;
    const trackArtistChanged = applyTrackArtist && Boolean(trackArtistValue);
    return titleChanged || albumArtistChanged || yearChanged || trackArtistChanged;
  }, [
    titleValue,
    albumArtistValue,
    yearValue,
    trackArtistValue,
    applyTrackArtist,
    initialTitle,
    initialAlbumArtist,
    initialYear
  ]);

  const handleSave = async () => {
    if (!albumId) return;
    const yearValue = parseOptionalInt(year);
    if (year.trim() && yearValue === undefined) {
      setTagsError("Year must be a valid number.");
      return;
    }
    if (applyTrackArtist && !trackArtist.trim()) {
      setTagsError("Track artist is required when applying to all tracks.");
      return;
    }
    if (!hasTagChanges) return;

    const payload: Record<string, string | number> = { album_id: albumId };
    const titleValue = title.trim();
    const albumArtistValue = albumArtist.trim();
    const trackArtistValue = trackArtist.trim();
    if (titleValue) payload.album = titleValue;
    if (albumArtistValue) payload.album_artist = albumArtistValue;
    if (yearValue !== undefined) payload.year = yearValue;
    if (applyTrackArtist && trackArtistValue) payload.track_artist = trackArtistValue;

    if (Object.keys(payload).length === 1) {
      setTagsError("Enter at least one field to update.");
      return;
    }

    setSaving(true);
    setTagsError(null);
    try {
      await onBeforeUpdate?.();
      const response = await postJson<AlbumMetadataUpdateResponse>(
        "/albums/metadata/update",
        payload
      );
      const updatedAlbumId = response?.album_id ?? albumId;
      onUpdated?.(updatedAlbumId);
      onClose();
    } catch (err) {
      setTagsError((err as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const handleSearch = async () => {
    if (!canSearch) return;
    setSearching(true);
    setMatchError(null);
    setSelectedIndex(null);
    setExpandedIndex(null);
    try {
      const response = await postJson<MusicBrainzMatchSearchResponse>(
        "/metadata/match/search",
        {
          kind: "album",
          title: matchTitle.trim(),
          artist: matchArtist.trim(),
          limit: 10
        }
      );
      setResults(response?.items ?? []);
    } catch (err) {
      setMatchError((err as Error).message);
    } finally {
      setSearching(false);
    }
  };

  const handleApply = async () => {
    if (!selected || !hasRequiredIds || !albumId) return;
    setApplying(true);
    setMatchError(null);
    try {
      await onBeforeUpdate?.();
      await postJson("/metadata/match/apply", {
        kind: "album",
        album_id: albumId,
        album_mbid: selected.release_mbid,
        artist_mbid: selected.artist_mbid ?? undefined,
        release_year: selected.year ?? undefined,
        override_existing: true
      });
      onUpdated?.(albumId);
      onClose();
    } catch (err) {
      setMatchError((err as Error).message);
    } finally {
      setApplying(false);
    }
  };

  return (
    <Modal open={open} title="Album metadata" onClose={onClose}>
      <div className="modal-tabs" role="tablist" aria-label="Album metadata sections">
        <button
          type="button"
          className={`modal-tab${activeTab === "tags" ? " active" : ""}`}
          onClick={() => setActiveTab("tags")}
          role="tab"
          aria-selected={activeTab === "tags"}
        >
          General
        </button>
        <button
          type="button"
          className={`modal-tab${activeTab === "match" ? " active" : ""}`}
          onClick={() => setActiveTab("match")}
          role="tab"
          aria-selected={activeTab === "match"}
        >
          MusicBrainz
        </button>
      </div>

      {activeTab === "tags" ? (
        <div className="track-meta" role="tabpanel">
          <div className="track-meta-target">
            <span className="muted small">Target</span>
            <div className="track-meta-title">{targetLabel}</div>
            <div className="muted small track-meta-note">
              Updates all tracks in this album. Leave a field blank to keep existing tags.
            </div>
          </div>

          <div className="track-meta-form">
            <label className="track-meta-field">
              <span className="muted small">Album title</span>
              <input
                className="track-meta-input"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                disabled={loading}
              />
            </label>
            <label className="track-meta-field">
              <span className="muted small">Album artist</span>
              <input
                className="track-meta-input"
                value={albumArtist}
                onChange={(event) => setAlbumArtist(event.target.value)}
                disabled={loading}
              />
            </label>
            <label className="track-meta-field">
              <span className="muted small">Year</span>
              <input
                className="track-meta-input"
                inputMode="numeric"
                value={year}
                onChange={(event) => setYear(event.target.value)}
                disabled={loading}
              />
            </label>
          </div>

          <div className="track-meta-form">
            <label className="track-meta-field">
              <span className="muted small">Track artist (optional)</span>
              <input
                className="track-meta-input"
                value={trackArtist}
                onChange={(event) => setTrackArtist(event.target.value)}
                disabled={loading || !applyTrackArtist}
                placeholder="Leave off for compilations"
              />
            </label>
            <label className="track-meta-field">
              <span className="muted small">Apply track artist</span>
              <button
                type="button"
                className={`btn ghost small${applyTrackArtist ? " active" : ""}`}
                onClick={() => setApplyTrackArtist((prev) => !prev)}
                disabled={loading}
                aria-pressed={applyTrackArtist}
              >
                {applyTrackArtist ? "On" : "Off"}
              </button>
            </label>
          </div>

          <div className="track-meta-actions">
            {tagsError ? <div className="alert">{tagsError}</div> : null}
            <button
              className="btn"
              onClick={handleSave}
              disabled={
                saving
                || loading
                || !hasTagChanges
                || !isYearValid
                || !isTrackArtistValid
              }
            >
              {saving ? "Saving..." : "Save to files"}
            </button>
          </div>
        </div>
      ) : null}

      {activeTab === "match" ? (
        <div className="mb-match" role="tabpanel">
          <div className="mb-match-target">
            <span className="muted small">Target</span>
            <div className="mb-match-target-title">{targetLabel}</div>
          </div>

          <div className="mb-match-form">
            <label className="mb-match-field">
              <span className="muted small">Album title</span>
              <input
                className="mb-match-input"
                value={matchTitle}
                onChange={(event) => setMatchTitle(event.target.value)}
              />
            </label>
            <label className="mb-match-field">
              <span className="muted small">Artist</span>
              <input
                className="mb-match-input"
                value={matchArtist}
                onChange={(event) => setMatchArtist(event.target.value)}
              />
            </label>
          </div>

          <div className="mb-match-actions">
            <button
              className="btn ghost"
              onClick={handleSearch}
              disabled={!canSearch || searching}
            >
              {searching ? "Searching..." : "Search"}
            </button>
            <button className="btn" onClick={handleApply} disabled={!hasRequiredIds || applying}>
              {applying ? "Applying..." : "Use match"}
            </button>
          </div>

          {matchError ? <div className="alert">{matchError}</div> : null}

          <div className="mb-match-results">
            {results.map((item, index) => (
              <button
                key={`${item.release_mbid ?? item.title}-${index}`}
                type="button"
                className={`mb-match-item${selectedIndex === index ? " selected" : ""}`}
                onClick={() => {
                  setSelectedIndex(index);
                  setExpandedIndex((prev) => (prev === index ? null : index));
                }}
              >
                <div className="mb-match-item-main">
                  {item.release_mbid ? (
                    <img
                      className="mb-match-thumb"
                      src={`https://coverartarchive.org/release/${item.release_mbid}/front-250`}
                      alt=""
                      aria-hidden="true"
                      loading="lazy"
                    />
                  ) : null}
                  <div className="mb-match-item-title">{item.title}</div>
                  <div className="muted small">
                    {item.artist}
                    {item.release_title ? ` · ${item.release_title}` : ""}
                  </div>
                </div>
                <div className="mb-match-item-meta">
                  {item.year ? <span>{item.year}</span> : <span>—</span>}
                  {item.score !== null && item.score !== undefined ? (
                    <span>{item.score}</span>
                  ) : (
                    <span>—</span>
                  )}
                </div>
                {expandedIndex === index ? (
                  <div className="mb-match-item-details">
                    <div className="mb-match-detail">
                      <span className="mb-match-detail-label">Release</span>
                      <span>{item.release_title ?? "—"}</span>
                    </div>
                    <div className="mb-match-detail">
                      <span className="mb-match-detail-label">Release MBID</span>
                      <span>{item.release_mbid ?? "—"}</span>
                    </div>
                    <div className="mb-match-detail">
                      <span className="mb-match-detail-label">Artist MBID</span>
                      <span>{item.artist_mbid ?? "—"}</span>
                    </div>
                  </div>
                ) : null}
              </button>
            ))}
            {!searching && results.length === 0 ? (
              <div className="muted small">No results yet. Try refining the query.</div>
            ) : null}
          </div>
        </div>
      ) : null}
    </Modal>
  );
}
