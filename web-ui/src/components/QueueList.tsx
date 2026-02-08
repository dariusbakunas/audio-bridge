import { QueueItem } from "../types";
import { apiUrl } from "../api";

interface QueueListProps {
  items: QueueItem[];
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  onPlayFrom: (path: string) => void;
}

export default function QueueList({
  items,
  formatMs,
  placeholder,
  canPlay,
  onPlayFrom
}: QueueListProps) {
  return (
    <div className="queue-list">
      {items.map((item, index) => {
        const fallback = item.kind === "track" ? placeholder(item.album, item.artist) : "";
        const coverUrl = item.kind === "track"
          ? apiUrl(`/art?path=${encodeURIComponent(item.path)}`)
          : "";
        return (
          <div key={`${item.kind}-${index}`} className="queue-row">
            {item.kind === "track" ? (
              <>
                <div className="queue-main">
                  <div className="queue-cover-frame">
                    <img
                      className="queue-cover"
                      src={coverUrl}
                      alt=""
                      aria-hidden="true"
                      onError={(event) => {
                        const img = event.currentTarget;
                        if (img.src !== fallback) {
                          img.onerror = null;
                          img.src = fallback;
                        }
                      }}
                    />
                    <button
                      className="queue-play"
                      type="button"
                      aria-label={`Play ${item.file_name}`}
                      title="Play from queue"
                      disabled={!canPlay}
                      onClick={() => onPlayFrom(item.path)}
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M8 5.5v13l11-6.5-11-6.5Z" fill="currentColor" />
                      </svg>
                    </button>
                  </div>
                  <div>
                    <div className="queue-title">{item.file_name}</div>
                    <div className="muted small">
                      {item.artist ?? "Unknown artist"}
                      {item.album ? ` - ${item.album}` : ""}
                    </div>
                  </div>
                </div>
                <div className="queue-meta">
                  <span>{item.format}</span>
                  <span>{formatMs(item.duration_ms)}</span>
                </div>
              </>
            ) : (
              <span className="muted">Missing: {item.path}</span>
            )}
          </div>
        );
      })}
      {items.length === 0 ? (
        <p className="muted">Queue is empty. Add tracks from the TUI for now.</p>
      ) : null}
    </div>
  );
}
