import { AlbumSummary, TrackSummary } from "../types";

interface AlbumDetailViewProps {
  album: AlbumSummary | null;
  tracks: TrackSummary[];
  loading: boolean;
  error: string | null;
  placeholder: string;
  canPlay: boolean;
  formatMs: (ms?: number | null) => string;
  onBack: () => void;
  onPlayAlbum: () => void;
  onPlayTrack: (track: TrackSummary) => void;
  onQueueTrack: (track: TrackSummary) => void;
}

export default function AlbumDetailView({
  album,
  tracks,
  loading,
  error,
  placeholder,
  canPlay,
  formatMs,
  onBack,
  onPlayAlbum,
  onPlayTrack,
  onQueueTrack
}: AlbumDetailViewProps) {
  return (
    <section className="album-view">
      <div className="album-header">
        <button className="btn ghost small" onClick={onBack}>
          Back to albums
        </button>
      </div>
      <div className="card album-detail">
        <div className="album-detail-top">
          <div className="album-detail-left">
            <img
              className="album-cover large"
              src={album?.cover_art_url ?? placeholder}
              alt={album?.title ?? "Album art"}
            />
          </div>
          <div className="album-detail-right">
            <div className="album-meta">
              <div className="eyebrow">Album</div>
              <h2>{album?.title ?? "Unknown album"}</h2>
              <div className="muted">{album?.artist ?? "Unknown artist"}</div>
              <div className="muted small">
                {album?.year ? `${album.year} · ` : ""}
                {album?.track_count ?? tracks.length} tracks
              </div>
              <div className="muted small">{album?.mbid ? `MBID: ${album.mbid}` : "MBID: —"}</div>
              <div className="muted small">
                {album?.cover_art_url
                  ? "Cover: cached"
                  : album?.mbid
                    ? "Cover: not cached"
                    : "Cover: unavailable"}
              </div>
              <button className="btn ghost small" onClick={onPlayAlbum} disabled={!canPlay}>
                Play album
              </button>
            </div>
          </div>
        </div>
        <div className="album-tracklist">
          {loading ? <p className="muted">Loading tracks...</p> : null}
          {error ? <p className="muted">{error}</p> : null}
          {!loading && !error ? (
            <div className="album-tracks">
              {tracks.map((track) => (
                <div key={track.id} className="album-track-row">
                  <div>
                    <div className="album-track-title">
                      {track.track_number ? `${track.track_number}. ` : ""}
                      {track.title ?? track.file_name}
                    </div>
                    <div className="muted small">{track.artist ?? "Unknown artist"}</div>
                  </div>
                  <div className="album-track-actions">
                    <span className="muted small">{formatMs(track.duration_ms)}</span>
                    <button
                      className="btn ghost small"
                      onClick={() => onPlayTrack(track)}
                      disabled={!canPlay}
                    >
                      Play
                    </button>
                    <button className="btn ghost small" onClick={() => onQueueTrack(track)}>
                      Queue
                    </button>
                  </div>
                </div>
              ))}
              {tracks.length === 0 ? (
                <div className="muted small">No tracks found for this album.</div>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}
