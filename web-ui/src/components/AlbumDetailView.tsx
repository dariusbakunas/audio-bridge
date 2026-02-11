import { AlbumSummary, TrackSummary } from "../types";
import hiResBadge from "../assets/hi-res.png";
import TrackMenu from "./TrackMenu";

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
  trackMenuPath: string | null;
  trackMenuPosition: { top: number; right: number } | null;
  onToggleMenu: (path: string, target: Element) => void;
  onMenuPlay: (path: string) => void;
  onMenuQueue: (path: string) => void;
  onMenuPlayNext: (path: string) => void;
  onMenuRescan: (path: string) => void;
  onFixTrackMatch: (path: string) => void;
  onEditTrackMetadata: (path: string) => void;
  onEditAlbumMetadata: () => void;
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
  trackMenuPath,
  trackMenuPosition,
  onToggleMenu,
  onMenuPlay,
  onMenuQueue,
  onMenuPlayNext,
  onMenuRescan,
  onFixTrackMatch,
  onEditTrackMetadata,
  onEditAlbumMetadata
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
                src={
                  album?.cover_art_url
                    ? `${album.cover_art_url}${album.cover_art_path ? `?v=${encodeURIComponent(album.cover_art_path)}` : ""}`
                    : placeholder(album?.title, album?.artist)
                }
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
              <h2 className="album-detail-title">
                <span className="album-detail-title-text">
                  {album?.title ?? "Unknown album"}
                </span>
                {album?.hi_res ? (
                  <span
                    className="hires-badge detail"
                    style={{ backgroundImage: `url(${hiResBadge})` }}
                    role="img"
                    aria-label="Hi-res audio"
                    title="Hi-res audio (24-bit)"
                  />
                ) : null}
              </h2>
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
              <div className="album-meta-actions">
                <button
                  className="icon-btn small"
                  type="button"
                  onClick={onEditAlbumMetadata}
                  disabled={!album}
                  aria-label="Edit album metadata"
                  title="Edit album metadata"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M3 17.25V21h3.75l11-11-3.75-3.75-11 11z" />
                    <path d="M20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </div>
        <div className="album-tracklist">
          {loading ? <p className="muted">Loading tracks...</p> : null}
          {error ? <p className="muted">{error}</p> : null}
          {!loading && !error ? (
            <div className="album-tracks">
              {tracks.map((track, index) => {
                const prevDisc = index > 0 ? tracks[index - 1]?.disc_number ?? null : null;
                const disc = track.disc_number ?? null;
                const showDiscHeader = disc !== null && disc !== prevDisc;
                const menuOpen = trackMenuPath === track.path;
                const menuStyle = menuOpen && trackMenuPosition
                  ? { top: trackMenuPosition.top, right: trackMenuPosition.right }
                  : undefined;
                return (
                  <div key={track.id} className="album-track-block">
                    {showDiscHeader ? (
                      <div className="album-disc-header">
                        <span className="pill small">Disc {disc}</span>
                      </div>
                    ) : null}
                    <div className="album-track-row">
                      <div className="album-track-main">
                        <button
                          className="track-play-btn"
                          type="button"
                          onClick={() => onPlayTrack(track)}
                          disabled={!canPlay}
                          aria-label={`Play ${track.title ?? track.file_name}`}
                          title="Play track"
                        >
                          <span className="track-index">{track.track_number ?? ""}</span>
                          <svg className="track-play-icon" viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M8 5.5v13l11-6.5-11-6.5Z" fill="currentColor" />
                          </svg>
                        </button>
                        <div>
                          <div className="album-track-title">{track.title ?? track.file_name}</div>
                          <div className="muted small">{track.artist ?? "Unknown artist"}</div>
                        </div>
                      </div>
                      <div className="album-track-actions">
                        <span className="muted small">{formatMs(track.duration_ms)}</span>
                        <TrackMenu
                          open={menuOpen}
                          canPlay={canPlay}
                          menuStyle={menuStyle}
                          onToggle={(event) => onToggleMenu(track.path, event.currentTarget)}
                          onPlay={() => onMenuPlay(track.path)}
                          onQueue={() => onMenuQueue(track.path)}
                          onPlayNext={() => onMenuPlayNext(track.path)}
                          onFixMatch={() => onFixTrackMatch(track.path)}
                          onEditMetadata={() => onEditTrackMetadata(track.path)}
                          onRescan={() => onMenuRescan(track.path)}
                        />
                      </div>
                    </div>
                  </div>
                );
              })}
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
