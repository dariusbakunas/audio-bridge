import { QueueItem } from "../types";

interface QueueListProps {
  items: QueueItem[];
  formatMs: (ms?: number | null) => string;
}

export default function QueueList({ items, formatMs }: QueueListProps) {
  return (
    <div className="queue-list">
      {items.map((item, index) => (
        <div key={`${item.kind}-${index}`} className="queue-row">
          {item.kind === "track" ? (
            <>
              <div>
                <div className="queue-title">{item.file_name}</div>
                <div className="muted small">
                  {item.artist ?? "Unknown artist"}
                  {item.album ? ` - ${item.album}` : ""}
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
      ))}
      {items.length === 0 ? (
        <p className="muted">Queue is empty. Add tracks from the TUI for now.</p>
      ) : null}
    </div>
  );
}
