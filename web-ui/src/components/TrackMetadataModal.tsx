import { useEffect, useState } from "react";
import { fetchJson, postJson } from "../api";
import { TrackMetadataFieldsResponse, TrackMetadataResponse } from "../types";
import Modal from "./Modal";

const CORE_FIELDS = new Set([
  "title",
  "artist",
  "album",
  "album_artist",
  "year",
  "track_number",
  "disc_number"
]);

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
  const [extraTags, setExtraTags] = useState<Record<string, string>>({});
  const [enabledFields, setEnabledFields] = useState<Record<string, boolean>>({});

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
    setExtraTags({});
    setEnabledFields({});

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
          setExtraTags(response.extra_tags ?? {});
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
    if (isEnabled("year") && year.trim() && yearValue === undefined) {
      setError("Year must be a valid number.");
      return;
    }
    if (isEnabled("track_number") && trackNumber.trim() && trackValue === undefined) {
      setError("Track number must be a valid number.");
      return;
    }
    if (isEnabled("disc_number") && discNumber.trim() && discValue === undefined) {
      setError("Disc number must be a valid number.");
      return;
    }

    const payload: Record<string, unknown> = trackId
      ? { track_id: trackId }
      : { path: trackPath ?? "" };
    const clearFields: string[] = [];
    const titleValue = title.trim();
    const artistValue = artist.trim();
    const albumValue = album.trim();
    const albumArtistValue = albumArtist.trim();
    if (isEnabled("title")) {
      if (titleValue) payload.title = titleValue;
      else clearFields.push("title");
    }
    if (isEnabled("artist")) {
      if (artistValue) payload.artist = artistValue;
      else clearFields.push("artist");
    }
    if (isEnabled("album")) {
      if (albumValue) payload.album = albumValue;
      else clearFields.push("album");
    }
    if (isEnabled("album_artist")) {
      if (albumArtistValue) payload.album_artist = albumArtistValue;
      else clearFields.push("album_artist");
    }
    if (isEnabled("year")) {
      if (yearValue !== undefined) payload.year = yearValue;
      else clearFields.push("year");
    }
    if (isEnabled("track_number")) {
      if (trackValue !== undefined) payload.track_number = trackValue;
      else clearFields.push("track_number");
    }
    if (isEnabled("disc_number")) {
      if (discValue !== undefined) payload.disc_number = discValue;
      else clearFields.push("disc_number");
    }
    const editableExtraTags = Object.entries(extraTags).reduce<Record<string, string>>((acc, [key, value]) => {
      const normalizedKey = key.trim().toUpperCase();
      const normalizedValue = value.trim();
      if (!normalizedKey || !normalizedValue) return acc;
      if (CORE_FIELDS.has(normalizedKey.toLowerCase())) return acc;
      if (!isEnabled(normalizedKey)) return acc;
      acc[normalizedKey] = normalizedValue;
      return acc;
    }, {});
    const clearExtraTags = editableExtraKeys
      .filter((key) => isEnabled(key))
      .filter((key) => (extraTags[key] ?? "").trim().length === 0);
    if (Object.keys(editableExtraTags).length > 0) {
      payload.extra_tags = editableExtraTags;
    }
    if (clearFields.length > 0) {
      payload.clear_fields = clearFields;
    }
    if (clearExtraTags.length > 0) {
      payload.clear_extra_tags = clearExtraTags;
    }

    if (Object.keys(payload).length === 1) {
      setError("Enable at least one field to update.");
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
  const isEnabled = (key: string) => Boolean(enabledFields[key]);
  const toggleEnabled = (key: string, enabled: boolean) => {
    setEnabledFields((current) => ({ ...current, [key]: enabled }));
  };
  const editableExtraKeys = Array.from(
    new Set(
      (supportedFields ?? [])
        .map((field) => field.trim())
        .filter((field) => field.length > 0)
        .filter((field) => !CORE_FIELDS.has(field.toLowerCase()))
        .map((field) => field.toUpperCase())
        .concat(Object.keys(extraTags).map((field) => field.trim().toUpperCase()))
        .filter((field) => field.length > 0)
    )
  ).sort();
  const setExtraTagValue = (key: string, value: string) => {
    setExtraTags((current) => ({ ...current, [key]: value }));
  };

  return (
    <Modal open={open} title="Edit track metadata" onClose={onClose}>
      <div className="track-meta">
        <div className="track-meta-target">
          <span className="muted small">Target</span>
          <div className="track-meta-title">{targetLabel}</div>
          <div className="muted small track-meta-note">
            Check fields you want to write. Checked empty fields are cleared.
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
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("title")}
                  onChange={(event) => toggleEnabled("title", event.target.checked)}
                  disabled={loading}
                />
                <span>Track title</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              disabled={loading || !isEnabled("title")}
            />
          </label>
          ) : null}
          {supportsField("artist") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("artist")}
                  onChange={(event) => toggleEnabled("artist", event.target.checked)}
                  disabled={loading}
                />
                <span>Artist</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              value={artist}
              onChange={(event) => setArtist(event.target.value)}
              disabled={loading || !isEnabled("artist")}
            />
          </label>
          ) : null}
          {supportsField("album") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("album")}
                  onChange={(event) => toggleEnabled("album", event.target.checked)}
                  disabled={loading}
                />
                <span>Album</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              value={album}
              onChange={(event) => setAlbum(event.target.value)}
              disabled={loading || !isEnabled("album")}
            />
          </label>
          ) : null}
          {supportsField("album_artist") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("album_artist")}
                  onChange={(event) => toggleEnabled("album_artist", event.target.checked)}
                  disabled={loading}
                />
                <span>Album artist</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              value={albumArtist}
              onChange={(event) => setAlbumArtist(event.target.value)}
              disabled={loading || !isEnabled("album_artist")}
            />
          </label>
          ) : null}
          {supportsField("year") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("year")}
                  onChange={(event) => toggleEnabled("year", event.target.checked)}
                  disabled={loading}
                />
                <span>Year</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              inputMode="numeric"
              value={year}
              onChange={(event) => setYear(event.target.value)}
              disabled={loading || !isEnabled("year")}
            />
          </label>
          ) : null}
          {supportsField("track_number") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("track_number")}
                  onChange={(event) => toggleEnabled("track_number", event.target.checked)}
                  disabled={loading}
                />
                <span>Track #</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              inputMode="numeric"
              value={trackNumber}
              onChange={(event) => setTrackNumber(event.target.value)}
              disabled={loading || !isEnabled("track_number")}
            />
          </label>
          ) : null}
          {supportsField("disc_number") ? (
            <label className="track-meta-field">
            <span className="muted small track-meta-field-head">
              <label className="track-meta-apply">
                <input
                  type="checkbox"
                  checked={isEnabled("disc_number")}
                  onChange={(event) => toggleEnabled("disc_number", event.target.checked)}
                  disabled={loading}
                />
                <span>Disc #</span>
              </label>
            </span>
            <input
              className="track-meta-input"
              inputMode="numeric"
              value={discNumber}
              onChange={(event) => setDiscNumber(event.target.value)}
              disabled={loading || !isEnabled("disc_number")}
            />
          </label>
          ) : null}
          {editableExtraKeys.map((key) => (
            <label className="track-meta-field" key={key}>
              <span className="muted small track-meta-field-head">
                <label className="track-meta-apply">
                  <input
                    type="checkbox"
                    checked={isEnabled(key)}
                    onChange={(event) => toggleEnabled(key, event.target.checked)}
                    disabled={loading}
                  />
                  <span>{key}</span>
                </label>
              </span>
              <input
                className="track-meta-input"
                value={extraTags[key] ?? ""}
                onChange={(event) => setExtraTagValue(key, event.target.value)}
                disabled={loading || !isEnabled(key)}
              />
            </label>
          ))}
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
