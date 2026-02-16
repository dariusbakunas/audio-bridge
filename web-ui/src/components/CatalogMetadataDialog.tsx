import { useEffect, useMemo, useState } from "react";
import { fetchJson, postJson } from "../api";
import { AlbumProfileResponse, MediaAssetInfo } from "../types";
import Modal from "./Modal";

const DEFAULT_LANG = "en-US";

type CatalogMetadataDialogProps = {
  open: boolean;
  albumId: number | null;
  albumTitle: string;
  artistName?: string | null;
  onClose: () => void;
  onUpdated?: (payload: { album?: AlbumProfileResponse }) => void;
};

function resolveAssetUrl(asset?: MediaAssetInfo | null): string | null {
  if (!asset?.url) return null;
  const version = asset.checksum ? `?v=${encodeURIComponent(asset.checksum)}` : "";
  return `${asset.url}${version}`;
}

export default function CatalogMetadataDialog({
  open,
  albumId,
  albumTitle,
  artistName,
  onClose,
  onUpdated
}: CatalogMetadataDialogProps) {
  const [albumNotes, setAlbumNotes] = useState("");
  const [initialAlbumNotes, setInitialAlbumNotes] = useState("");
  const [cachedAlbumImage, setCachedAlbumImage] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setAlbumNotes("");
    setInitialAlbumNotes("");
    setCachedAlbumImage(null);
    setError(null);

    if (!albumId) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const albumPromise = albumId
      ? fetchJson<AlbumProfileResponse>(`/albums/profile?album_id=${albumId}&lang=${DEFAULT_LANG}`)
      : Promise.resolve(null as AlbumProfileResponse | null);

    Promise.allSettled([albumPromise])
      .then(([albumResult]) => {
        if (albumResult.status === "fulfilled" && albumResult.value) {
          const album = albumResult.value;
          const notes = album.notes?.text ?? "";
          setAlbumNotes(notes);
          setInitialAlbumNotes(notes);
          setCachedAlbumImage(resolveAssetUrl(album.image));
        }
        if (albumResult.status === "rejected") {
          setError(
            albumResult.reason instanceof Error
              ? albumResult.reason.message
              : String(albumResult.reason)
          );
        }
      })
      .finally(() => {
        setLoading(false);
      });
  }, [open, albumId]);

  const notesChanged = useMemo(() => albumNotes.trim() !== initialAlbumNotes.trim(), [albumNotes, initialAlbumNotes]);

  const handleSave = async () => {
    if (!albumId) return;
    if (!notesChanged) return;
    setSaving(true);
    setError(null);

    try {
      let updatedAlbum: AlbumProfileResponse | undefined;
      if (albumId && notesChanged) {
        const payload: Record<string, string | number | boolean> = {
          album_id: albumId,
          lang: DEFAULT_LANG,
          source: "manual"
        };
        if (notesChanged) payload.notes = albumNotes.trim();
        if (notesChanged && albumNotes.trim()) payload.notes_locked = true;
        updatedAlbum = await postJson<AlbumProfileResponse>("/albums/profile/update", payload);
      }

      if (albumId && !updatedAlbum) {
        updatedAlbum = await fetchJson<AlbumProfileResponse>(
          `/albums/profile?album_id=${albumId}&lang=${DEFAULT_LANG}`
        );
      }

      onUpdated?.({ album: updatedAlbum });
      onClose();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title="Catalog metadata"
      onClose={onClose}
      className="catalog-meta-modal"
    >
      <div className="catalog-meta">
        <div className="catalog-meta-target">
          <span className="muted small">Album</span>
          <div className="catalog-meta-title-line">
            <span>{albumTitle || "Unknown album"}</span>
            {artistName ? (
              <>
                <span className="catalog-meta-sep">•</span>
                <span className="catalog-meta-artist">{artistName}</span>
              </>
            ) : null}
          </div>
        </div>

        {loading ? <p className="muted">Loading catalog metadata...</p> : null}
        {error ? <p className="muted">{error}</p> : null}

        {!loading ? (
          <div className="catalog-meta-grid">
            <div className="catalog-meta-section">
              <div className="catalog-meta-section-title">Album notes</div>
              <textarea
                className="catalog-meta-textarea"
                value={albumNotes}
                onChange={(event) => setAlbumNotes(event.target.value)}
                placeholder="Add album notes (offline cached)"
              />
              {cachedAlbumImage ? (
                <div className="catalog-meta-image">
                  <img src={cachedAlbumImage} alt="Album asset" />
                </div>
              ) : null}
            </div>
          </div>
        ) : null}

        <div className="modal-actions">
          <button className="btn ghost" type="button" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          <button
            className="btn"
            type="button"
            onClick={handleSave}
            disabled={saving || loading || !notesChanged}
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
