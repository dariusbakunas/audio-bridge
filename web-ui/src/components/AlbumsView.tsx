import { AlbumSummary } from "../types";

interface AlbumsViewProps {
  albums: AlbumSummary[];
  loading: boolean;
  error: string | null;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  activeAlbumId: number | null;
  isPlaying: boolean;
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
  onSelectAlbum,
  onPlayAlbum,
  onPause
}: AlbumsViewProps) {
  return (
    <div className="card">
      <div className="card-header actions-only">
        <div className="card-actions">
          <span className="pill">{albums.length} albums</span>
        </div>
      </div>
      {loading ? <p className="muted">Loading albums...</p> : null}
      {error ? <p className="muted">{error}</p> : null}
      {!loading && !error ? (
        <div className="album-grid">
          {albums.map((album) => (
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
                  <img
                    className="album-cover"
                    src={album.cover_art_url ?? placeholder(album.title, album.artist)}
                    alt={album.title}
                    loading="lazy"
                  />
                  <button
                    className={`album-play${
                      activeAlbumId === album.id && isPlaying ? " is-active" : ""
                    }`}
                    type="button"
                    onClick={(event) => {
                      event.stopPropagation();
                      if (activeAlbumId === album.id && isPlaying) {
                        onPause();
                        return;
                      }
                      onPlayAlbum(album.id);
                    }}
                    disabled={!canPlay}
                    aria-label={
                      activeAlbumId === album.id && isPlaying ? "Pause playback" : `Play ${album.title}`
                    }
                    title={activeAlbumId === album.id && isPlaying ? "Pause" : "Play album"}
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      {activeAlbumId === album.id && isPlaying ? (
                        <path d="M7 5h4v14H7zM13 5h4v14h-4z" fill="currentColor" />
                      ) : (
                        <path d="M8 5.5v13l11-6.5-11-6.5Z" fill="currentColor" />
                      )}
                    </svg>
                  </button>
                </div>
                <div className="album-card-info">
                  <div className="album-title">{album.title}</div>
                  <div className="muted small">{album.artist ?? "Unknown artist"}</div>
                </div>
              </div>
            </div>
          ))}
          {albums.length === 0 ? <p className="muted">No albums found.</p> : null}
        </div>
      ) : null}
    </div>
  );
}
