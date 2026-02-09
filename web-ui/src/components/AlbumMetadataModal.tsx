import { useEffect, useState } from "react";
import { fetchJson, postJson } from "../api";
import { AlbumMetadataResponse, AlbumMetadataUpdateResponse } from "../types";
import Modal from "./Modal";

interface AlbumMetadataModalProps {
  open: boolean;
  albumId: number | null;
  targetLabel: string;
  defaults: {
    title?: string | null;
    albumArtist?: string | null;
    year?: number | null;
  };
  onClose: () => void;
  onSaved?: (albumId: number) => void;
}

function parseOptionalInt(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number.parseInt(trimmed, 10);
  if (Number.isNaN(parsed) || parsed <= 0) return undefined;
  return parsed;
}

export default function AlbumMetadataModal({
  open,
  albumId,
  targetLabel,
  defaults,
  onClose,
  onSaved
}: AlbumMetadataModalProps) {
  const [title, setTitle] = useState(defaults.title ?? "");
  const [albumArtist, setAlbumArtist] = useState(defaults.albumArtist ?? "");
  const [year, setYear] = useState(defaults.year ? String(defaults.year) : "");
  const [trackArtist, setTrackArtist] = useState("");
  const [applyTrackArtist, setApplyTrackArtist] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setTitle(defaults.title ?? "");
    setAlbumArtist(defaults.albumArtist ?? "");
    setYear(defaults.year ? String(defaults.year) : "");
    setTrackArtist("");
    setApplyTrackArtist(false);
    setError(null);

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
      })
      .catch((err) => {
        if (!active) return;
        setError((err as Error).message);
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [open, albumId, defaults.title, defaults.albumArtist, defaults.year]);

  const handleSave = async () => {
    if (!albumId) return;
    const yearValue = parseOptionalInt(year);
    if (year.trim() && yearValue === undefined) {
      setError("Year must be a valid number.");
      return;
    }
    if (applyTrackArtist && !trackArtist.trim()) {
      setError("Track artist is required when applying to all tracks.");
      return;
    }

    const payload: Record<string, string | number> = { album_id: albumId };
    const titleValue = title.trim();
    const albumArtistValue = albumArtist.trim();
    const trackArtistValue = trackArtist.trim();
    if (titleValue) payload.album = titleValue;
    if (albumArtistValue) payload.album_artist = albumArtistValue;
    if (yearValue !== undefined) payload.year = yearValue;
    if (applyTrackArtist && trackArtistValue) payload.track_artist = trackArtistValue;

    if (Object.keys(payload).length === 1) {
      setError("Enter at least one field to update.");
      return;
    }

    setSaving(true);
    setError(null);
    try {
      const response = await postJson<AlbumMetadataUpdateResponse>(
        "/albums/metadata/update",
        payload
      );
      const updatedAlbumId = response?.album_id ?? albumId;
      onSaved?.(updatedAlbumId);
      onClose();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal open={open} title="Edit album tags" onClose={onClose}>
      <div className="track-meta">
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
          {error ? <div className="alert">{error}</div> : null}
          <button className="btn" onClick={handleSave} disabled={saving || loading}>
            {saving ? "Saving..." : "Save to files"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
