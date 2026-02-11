import { useEffect, useMemo, useState } from "react";
import { postJson } from "../api";
import {
  MusicBrainzMatchCandidate,
  MusicBrainzMatchKind,
  MusicBrainzMatchSearchResponse
} from "../types";
import Modal from "./Modal";

interface MusicBrainzMatchModalProps {
  open: boolean;
  kind: MusicBrainzMatchKind;
  targetLabel: string;
  defaults: {
    title: string;
    artist: string;
    album?: string | null;
  };
  trackPath?: string | null;
  albumId?: number | null;
  onClose: () => void;
  onApplied?: () => void;
}

export default function MusicBrainzMatchModal({
  open,
  kind,
  targetLabel,
  defaults,
  trackPath,
  albumId,
  onClose,
  onApplied
}: MusicBrainzMatchModalProps) {
  const [title, setTitle] = useState(defaults.title);
  const [artist, setArtist] = useState(defaults.artist);
  const [album, setAlbum] = useState(defaults.album ?? "");
  const [results, setResults] = useState<MusicBrainzMatchCandidate[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTitle(defaults.title);
    setArtist(defaults.artist);
    setAlbum(defaults.album ?? "");
    setResults([]);
    setSelectedIndex(null);
    setExpandedIndex(null);
    setError(null);
  }, [open, defaults.title, defaults.artist, defaults.album, kind]);

  const canSearch = Boolean(title.trim() && artist.trim());
  const selected = selectedIndex === null ? null : results[selectedIndex] ?? null;
  const hasRequiredIds = useMemo(() => {
    if (!selected) return false;
    if (kind === "track") {
      return Boolean(selected.recording_mbid && trackPath);
    }
    return Boolean(selected.release_mbid && albumId !== null && albumId !== undefined);
  }, [selected, kind, trackPath, albumId]);

  const handleSearch = async () => {
    if (!canSearch) return;
    setLoading(true);
    setError(null);
    setSelectedIndex(null);
    setExpandedIndex(null);
    try {
      const body = {
        kind,
        title: title.trim(),
        artist: artist.trim(),
        album: kind === "track" && album.trim() ? album.trim() : undefined,
        limit: 10
      };
      const response = await postJson<MusicBrainzMatchSearchResponse>(
        "/metadata/match/search",
        body
      );
      setResults(response?.items ?? []);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  };

  const handleApply = async () => {
    if (!selected || !hasRequiredIds) return;
    setApplying(true);
    setError(null);
    try {
      if (kind === "track") {
        await postJson("/metadata/match/apply", {
          kind: "track",
          path: trackPath,
          recording_mbid: selected.recording_mbid,
          artist_mbid: selected.artist_mbid ?? undefined,
          album_mbid: selected.release_mbid ?? undefined,
          release_year: selected.year ?? undefined,
          override_existing: true
        });
      } else {
        await postJson("/metadata/match/apply", {
          kind: "album",
          album_id: albumId,
          album_mbid: selected.release_mbid,
          artist_mbid: selected.artist_mbid ?? undefined,
          release_year: selected.year ?? undefined,
          override_existing: true
        });
      }
      onApplied?.();
      onClose();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setApplying(false);
    }
  };

  return (
    <Modal
      open={open}
      title="Fix MusicBrainz match"
      onClose={onClose}
      headerRight={<span className="pill">{kind === "track" ? "Track" : "Album"}</span>}
    >
      <div className="mb-match">
        <div className="mb-match-target">
          <span className="muted small">Target</span>
          <div className="mb-match-target-title">{targetLabel}</div>
        </div>

        <div className="mb-match-form">
          <label className="mb-match-field">
            <span className="muted small">{kind === "track" ? "Track title" : "Album title"}</span>
            <input
              className="mb-match-input"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
            />
          </label>
          <label className="mb-match-field">
            <span className="muted small">Artist</span>
            <input
              className="mb-match-input"
              value={artist}
              onChange={(event) => setArtist(event.target.value)}
            />
          </label>
          {kind === "track" ? (
            <label className="mb-match-field">
              <span className="muted small">Album (optional)</span>
              <input
                className="mb-match-input"
                value={album}
                onChange={(event) => setAlbum(event.target.value)}
              />
            </label>
          ) : null}
        </div>

        <div className="mb-match-actions">
          <button className="btn ghost" onClick={handleSearch} disabled={!canSearch || loading}>
            {loading ? "Searching..." : "Search"}
          </button>
          <button
            className="btn"
            onClick={handleApply}
            disabled={!hasRequiredIds || applying}
          >
            {applying ? "Applying..." : "Use match"}
          </button>
        </div>

        {error ? <div className="alert">{error}</div> : null}

        <div className="mb-match-results">
          {results.map((item, index) => (
            <button
              key={`${item.recording_mbid ?? item.release_mbid ?? item.title}-${index}`}
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
                      <span className="mb-match-detail-label">Recording MBID</span>
                      <span>{item.recording_mbid ?? "—"}</span>
                    </div>
                    <div className="mb-match-detail">
                      <span className="mb-match-detail-label">Artist MBID</span>
                      <span>{item.artist_mbid ?? "—"}</span>
                    </div>
                    {item.release_mbid || item.recording_mbid ? (
                      <div className="mb-match-detail">
                        <span className="mb-match-detail-label">MusicBrainz</span>
                        <span className="mb-match-detail-links">
                          {item.release_mbid ? (
                            <a
                              className="mb-match-link"
                              href={`https://musicbrainz.org/release/${item.release_mbid}`}
                              target="_blank"
                              rel="noreferrer"
                            >
                              Release
                            </a>
                          ) : null}
                          {item.recording_mbid ? (
                            <a
                              className="mb-match-link"
                              href={`https://musicbrainz.org/recording/${item.recording_mbid}`}
                              target="_blank"
                              rel="noreferrer"
                            >
                              Recording
                            </a>
                          ) : null}
                        </span>
                      </div>
                    ) : null}
                  </div>
                ) : null}
            </button>
          ))}
          {!loading && results.length === 0 ? (
            <div className="muted small">No results yet. Try refining the query.</div>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}
