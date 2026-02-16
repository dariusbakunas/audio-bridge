import { useEffect, useState } from "react";
import { fetchJson, postJson } from "../api";
import { TrackMetadataFieldsResponse, TrackMetadataResponse } from "../types";
import Modal from "./Modal";

interface TrackMetadataModalProps {
  open: boolean;
  trackId?: number | null;
  trackPath?: string | null;
  targetLabel: string;
  defaults: {
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    year?: number | null;
    trackNumber?: number | null;
    discNumber?: number | null;
  };
  onClose: () => void;
  onSaved?: () => void;
}

function parseOptionalInt(value: string, allowZero = false): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number.parseInt(trimmed, 10);
  if (Number.isNaN(parsed)) return undefined;
  if (!allowZero && parsed <= 0) return undefined;
  return parsed;
}

export default function TrackMetadataModal({
  open,
  trackId,
  trackPath,
  targetLabel,
  defaults,
  onClose,
  onSaved
}: TrackMetadataModalProps) {
  const [title, setTitle] = useState(defaults.title ?? "");
  const [artist, setArtist] = useState(defaults.artist ?? "");
  const [album, setAlbum] = useState(defaults.album ?? "");
  const [albumArtist, setAlbumArtist] = useState(defaults.albumArtist ?? "");
  const [year, setYear] = useState(defaults.year ? String(defaults.year) : "");
  const [trackNumber, setTrackNumber] = useState(
    defaults.trackNumber ? String(defaults.trackNumber) : ""
  );
  const [discNumber, setDiscNumber] = useState(
    defaults.discNumber ? String(defaults.discNumber) : ""
  );
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [supportedFields, setSupportedFields] = useState<string[] | null>(null);
  const [tagType, setTagType] = useState<string>("");

  useEffect(() => {
    if (!open) return;
    let active = true;
    setTitle(defaults.title ?? "");
    setArtist(defaults.artist ?? "");
    setAlbum(defaults.album ?? "");
    setAlbumArtist(defaults.albumArtist ?? "");
    setYear(defaults.year ? String(defaults.year) : "");
    setTrackNumber(defaults.trackNumber ? String(defaults.trackNumber) : "");
    setDiscNumber(defaults.discNumber ? String(defaults.discNumber) : "");
    setError(null);
    setSupportedFields(null);
    setTagType("");

    if (!trackId && !trackPath) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const query = trackId
      ? `track_id=${trackId}`
      : `path=${encodeURIComponent(trackPath ?? "")}`;
    const metadataPromise = fetchJson<TrackMetadataResponse>(`/tracks/metadata?${query}`);
    const fieldsPromise = fetchJson<TrackMetadataFieldsResponse>(`/tracks/metadata/fields?${query}`);
    Promise.allSettled([metadataPromise, fieldsPromise])
      .then(([metadataResult, fieldsResult]) => {
        if (!active) return;
        if (metadataResult.status === "fulfilled" && metadataResult.value) {
          const response = metadataResult.value;
          setTitle(response.title ?? "");
          setArtist(response.artist ?? "");
          setAlbum(response.album ?? "");
          setAlbumArtist(response.album_artist ?? "");
          setYear(response.year ? String(response.year) : "");
          setTrackNumber(response.track_number ? String(response.track_number) : "");
          setDiscNumber(response.disc_number ? String(response.disc_number) : "");
        } else if (metadataResult.status === "rejected") {
          setError(metadataResult.reason instanceof Error ? metadataResult.reason.message : String(metadataResult.reason));
        }
        if (fieldsResult.status === "fulfilled" && fieldsResult.value) {
          setSupportedFields(fieldsResult.value.fields ?? []);
          setTagType(fieldsResult.value.tag_type ?? "");
        }
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [
    open,
    trackId,
    trackPath,
    defaults.title,
    defaults.artist,
    defaults.album,
    defaults.albumArtist,
    defaults.year,
    defaults.trackNumber,
    defaults.discNumber
  ]);

  const handleSave = async () => {
    if (!trackId && !trackPath) return;
    const yearValue = parseOptionalInt(year);
    const trackValue = parseOptionalInt(trackNumber);
    const discValue = parseOptionalInt(discNumber);
    if (year.trim() && yearValue === undefined) {
      setError("Year must be a valid number.");
      return;
    }
    if (trackNumber.trim() && trackValue === undefined) {
      setError("Track number must be a valid number.");
      return;
    }
    if (discNumber.trim() && discValue === undefined) {
      setError("Disc number must be a valid number.");
      return;
    }

    const payload: Record<string, string | number> = trackId
      ? { track_id: trackId }
      : { path: trackPath ?? "" };
    const titleValue = title.trim();
    const artistValue = artist.trim();
    const albumValue = album.trim();
    const albumArtistValue = albumArtist.trim();
    if (titleValue) payload.title = titleValue;
    if (artistValue) payload.artist = artistValue;
    if (albumValue) payload.album = albumValue;
    if (albumArtistValue) payload.album_artist = albumArtistValue;
    if (yearValue !== undefined) payload.year = yearValue;
    if (trackValue !== undefined) payload.track_number = trackValue;
    if (discValue !== undefined) payload.disc_number = discValue;

    if (Object.keys(payload).length === 1) {
      setError("Enter at least one field to update.");
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await postJson("/tracks/metadata/update", payload);
      onSaved?.();
      onClose();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const supportsField = (key: string) =>
    supportedFields === null || supportedFields.includes(key);

  return (
    <Modal open={open} title="Edit track metadata" onClose={onClose}>
      <div className="track-meta">
        <div className="track-meta-target">
          <span className="muted small">Target</span>
          <div className="track-meta-title">{targetLabel}</div>
          <div className="muted small track-meta-note">
            Leave a field blank to keep the existing tag value.
          </div>
          {tagType ? (
            <div className="muted small track-meta-note">
              Tag type: {tagType}
            </div>
          ) : null}
        </div>

        <div className="track-meta-form">
          {supportsField("title") ? (
            <label className="track-meta-field">
            <span className="muted small">Track title</span>
            <input
              className="track-meta-input"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
          {supportsField("artist") ? (
            <label className="track-meta-field">
            <span className="muted small">Artist</span>
            <input
              className="track-meta-input"
              value={artist}
              onChange={(event) => setArtist(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
          {supportsField("album") ? (
            <label className="track-meta-field">
            <span className="muted small">Album</span>
            <input
              className="track-meta-input"
              value={album}
              onChange={(event) => setAlbum(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
          {supportsField("album_artist") ? (
            <label className="track-meta-field">
            <span className="muted small">Album artist</span>
            <input
              className="track-meta-input"
              value={albumArtist}
              onChange={(event) => setAlbumArtist(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
          {supportsField("year") ? (
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
          ) : null}
          {supportsField("track_number") ? (
            <label className="track-meta-field">
            <span className="muted small">Track #</span>
            <input
              className="track-meta-input"
              inputMode="numeric"
              value={trackNumber}
              onChange={(event) => setTrackNumber(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
          {supportsField("disc_number") ? (
            <label className="track-meta-field">
            <span className="muted small">Disc #</span>
            <input
              className="track-meta-input"
              inputMode="numeric"
              value={discNumber}
              onChange={(event) => setDiscNumber(event.target.value)}
              disabled={loading}
            />
          </label>
          ) : null}
        </div>

        <div className="track-meta-actions">
          {error ? <div className="alert">{error}</div> : null}
          <button className="btn" onClick={handleSave} disabled={saving || loading}>
            {saving ? "Saving..." : "Save to file"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
