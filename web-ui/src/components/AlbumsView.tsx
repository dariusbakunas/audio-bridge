import { AlbumSummary } from "../types";

interface AlbumsViewProps {
  albums: AlbumSummary[];
  loading: boolean;
  error: string | null;
  placeholder: string;
  onSelectAlbum: (id: number) => void;
}

export default function AlbumsView({
  albums,
  loading,
  error,
  placeholder,
  onSelectAlbum
}: AlbumsViewProps) {
  return (
    <div className="card">
      <div className="card-header">
        <span>Albums</span>
        <div className="card-actions">
          <span className="pill">{albums.length} albums</span>
        </div>
      </div>
      {loading ? <p className="muted">Loading albums...</p> : null}
      {error ? <p className="muted">{error}</p> : null}
      {!loading && !error ? (
        <div className="album-grid">
          {albums.map((album) => (
            <button
              key={album.id}
              className="album-card"
              onClick={() => onSelectAlbum(album.id)}
            >
              <img
                className="album-cover"
                src={album.cover_art_url ?? placeholder}
                alt={album.title}
                loading="lazy"
              />
              <div className="album-card-info">
                <div className="album-title">{album.title}</div>
                <div className="muted small">{album.artist ?? "Unknown artist"}</div>
              </div>
            </button>
          ))}
          {albums.length === 0 ? <p className="muted">No albums found.</p> : null}
        </div>
      ) : null}
    </div>
  );
}
