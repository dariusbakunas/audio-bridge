import { AlbumSummary } from "../types";
import hiResBadge from "../assets/hi-res.png?q=1";
import { Pause, Play } from "lucide-react";

interface AlbumsViewProps {
  albums: AlbumSummary[];
  loading: boolean;
  error: string | null;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  activeAlbumId: number | null;
  isPlaying: boolean;
  isPaused: boolean;
  viewMode: "grid" | "list";
  onSelectAlbum: (id: number) => void;
  onPlayAlbum: (id: number) => void;
  onPause: () => void;
}

export default function AlbumsView({
  albums,
  loading,
  error,
  placeholder,
  canPlay,
  activeAlbumId,
  isPlaying,
  isPaused,
  viewMode,
  onSelectAlbum,
  onPlayAlbum,
  onPause
}: AlbumsViewProps) {
  return (
    <div className="albums-view">
      <div className="card-header actions-only">
        <div className="card-actions">
          <span className="pill">{albums.length} albums</span>
        </div>
      </div>
      {loading ? <p className="muted">Loading albums...</p> : null}
      {error ? <p className="muted">{error}</p> : null}
      {!loading && !error ? (
        viewMode === "grid" ? (
          <div className="album-grid">
            {albums.map((album) => {
              const displayYear = album.original_year ?? album.year ?? null;
              const coverSrc = album.cover_art_url
                ? `${album.cover_art_url}${album.cover_art_path ? `?v=${encodeURIComponent(album.cover_art_path)}` : ""}`
                : placeholder(album.title, album.artist);
              return (
                <div key={album.id} className="album-card">
                  <div
                    className="album-card-link"
                    role="button"
                    tabIndex={0}
                    onClick={() => onSelectAlbum(album.id)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        onSelectAlbum(album.id);
                      }
                    }}
                  >
                    <div className="album-cover-frame">
                      <img className="album-cover" src={coverSrc} alt={album.title} loading="lazy" />
                      <button
                        className={`album-play${
                          activeAlbumId === album.id && isPlaying ? " is-active" : ""
                        }`}
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          if (activeAlbumId === album.id && (isPlaying || isPaused)) {
                            onPause();
                            return;
                          }
                          onPlayAlbum(album.id);
                        }}
                        disabled={!canPlay}
                        aria-label={
                          activeAlbumId === album.id && (isPlaying || isPaused)
                            ? isPaused
                              ? "Resume playback"
                              : "Pause playback"
                            : `Play ${album.title}`
                        }
                        title={
                          activeAlbumId === album.id && (isPlaying || isPaused)
                            ? isPaused
                              ? "Resume"
                              : "Pause"
                            : "Play album"
                        }
                      >
                        {activeAlbumId === album.id && isPlaying ? (
                          <Pause className="icon" aria-hidden="true" />
                        ) : (
                          <Play className="icon" aria-hidden="true" />
                        )}
                      </button>
                    </div>
                    <div className="album-card-info">
                      <div className="album-title">
                        <span className="album-title-text">{album.title}</span>
                        {album.hi_res ? (
                          <span
                            className="hires-badge"
                            style={{ backgroundImage: `url(${hiResBadge})` }}
                            role="img"
                            aria-label="Hi-res audio"
                            title="Hi-res audio (24-bit)"
                          />
                        ) : null}
                      </div>
                      <div className="album-artist">{album.artist ?? "Unknown artist"}</div>
                      <div className="album-meta-row mono">
                        <span>{displayYear ?? "—"}</span>
                        <span className="meta-sep">•</span>
                        <span>{album.track_count} tracks</span>
                      </div>
                    </div>
                  </div>
                </div>
              );
            })}
            {albums.length === 0 ? <p className="muted">No albums found.</p> : null}
          </div>
        ) : (
          <div className="album-list">
            {albums.map((album) => {
              const displayYear = album.original_year ?? album.year ?? null;
              const coverSrc = album.cover_art_url
                ? `${album.cover_art_url}${album.cover_art_path ? `?v=${encodeURIComponent(album.cover_art_path)}` : ""}`
                : placeholder(album.title, album.artist);
              return (
                <div key={album.id} className="album-list-row">
                  <button
                    className="album-list-cover"
                    type="button"
                    onClick={() => onSelectAlbum(album.id)}
                    aria-label={`Open ${album.title}`}
                  >
                    <img className="album-list-image" src={coverSrc} alt={album.title} loading="lazy" />
                  </button>
                  <div className="album-list-info">
                    <div className="album-list-title">{album.title}</div>
                    <div className="muted small">{album.artist ?? "Unknown artist"}</div>
                    <div className="muted small mono">
                      {displayYear ? `${displayYear} · ` : ""}
                      {album.track_count ?? 0} tracks
                    </div>
                  </div>
                  <div className="album-list-actions">
                    <button
                      className="btn ghost small"
                      type="button"
                      onClick={() => onSelectAlbum(album.id)}
                    >
                      Open
                    </button>
                    <button
                      className="btn small"
                      type="button"
                      onClick={() => onPlayAlbum(album.id)}
                      disabled={!canPlay}
                    >
                      Play
                    </button>
                  </div>
                </div>
              );
            })}
            {albums.length === 0 ? <p className="muted">No albums found.</p> : null}
          </div>
        )
      ) : null}
    </div>
  );
}
