import { QueueItem } from "../types";
import { apiUrl } from "../api";
import { Pause, Play } from "lucide-react";

interface QueueListProps {
  items: QueueItem[];
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  isPaused: boolean;
  onPause: () => void;
  onPlayFrom: (payload: { trackId?: number; path?: string }) => void;
}

export default function QueueList({
  items,
  formatMs,
  placeholder,
  canPlay,
  isPaused,
  onPause,
  onPlayFrom
}: QueueListProps) {
  return (
    <div className="queue-list">
      {items.map((item, index) => {
        const isNowPlaying = item.kind === "track" ? Boolean(item.now_playing) : false;
        const PlaybackIcon = isNowPlaying ? (isPaused ? Play : Pause) : Play;
        const fallback = item.kind === "track" ? placeholder(item.album, item.artist) : "";
        const coverUrl = item.kind === "track"
          ? item.id
            ? apiUrl(`/tracks/${item.id}/cover`)
            : apiUrl(`/art?path=${encodeURIComponent(item.path)}`)
          : "";
        return (
          <div key={`${item.kind}-${index}`} className={`queue-row${isNowPlaying ? " is-playing" : ""}`}>
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
                    {isNowPlaying ? (
                      <div className="queue-playing-overlay" aria-hidden="true">
                        <div className={`queue-playing-indicator${isPaused ? " is-paused" : ""}`}>
                          <div className="equalizer-bar" />
                          <div className="equalizer-bar" />
                          <div className="equalizer-bar" />
                        </div>
                      </div>
                    ) : null}
                    <button
                      className="queue-play"
                      type="button"
                      aria-label={`Play ${item.file_name}`}
                      title={
                        isNowPlaying
                          ? isPaused
                            ? "Resume"
                            : "Pause"
                          : "Play from queue"
                      }
                      disabled={!canPlay}
                      onClick={() => {
                        if (isNowPlaying) {
                          onPause();
                          return;
                        }
                        onPlayFrom({
                          trackId: item.id ?? undefined,
                          path: item.id ? undefined : item.path
                        });
                      }}
                    >
                      <PlaybackIcon className="icon" aria-hidden="true" />
                    </button>
                  </div>
                  <div>
                    <div className="queue-title">
                      {item.file_name}
                    </div>
                    <div className="muted small">
                      {item.artist ?? "Unknown artist"}
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
