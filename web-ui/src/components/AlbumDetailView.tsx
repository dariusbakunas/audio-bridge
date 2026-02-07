import { AlbumSummary, TrackSummary } from "../types";

interface AlbumDetailViewProps {
  album: AlbumSummary | null;
  tracks: TrackSummary[];
  loading: boolean;
  error: string | null;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  formatMs: (ms?: number | null) => string;
  activeAlbumId: number | null;
  isPlaying: boolean;
  isPaused: boolean;
  onPause: () => void;
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
  activeAlbumId,
  isPlaying,
  isPaused,
  onPause,
  onPlayAlbum,
  onPlayTrack,
  onQueueTrack
}: AlbumDetailViewProps) {
  const isActive = Boolean(album?.id && activeAlbumId === album.id && (isPlaying || isPaused));
  const isActivePlaying = Boolean(album?.id && activeAlbumId === album.id && isPlaying);
  return (
    <section className="album-view">
      <div className="card album-detail">
        <div className="album-detail-top">
          <div className="album-detail-left">
            <div className="album-cover-frame large">
              <img
                className="album-cover large"
                src={album?.cover_art_url ?? placeholder(album?.title, album?.artist)}
                alt={album?.title ?? "Album art"}
              />
              <button
                className="album-play large"
                type="button"
                onClick={() => {
                  if (isActive) {
                    onPause();
                    return;
                  }
                  onPlayAlbum();
                }}
                disabled={!canPlay}
                aria-label={isActive ? (isPaused ? "Resume playback" : "Pause playback") : "Play album"}
                title={isActive ? (isPaused ? "Resume" : "Pause") : "Play album"}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  {isActivePlaying ? (
                    <path d="M7 5h4v14H7zM13 5h4v14h-4z" fill="currentColor" />
                  ) : (
                    <path d="M8 5.5v13l11-6.5-11-6.5Z" fill="currentColor" />
                  )}
                </svg>
              </button>
            </div>
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
