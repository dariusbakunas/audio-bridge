import { QueueItem } from "../types";
import QueueList from "./QueueList";
import { X } from "lucide-react";

interface QueueModalProps {
  open: boolean;
  items: QueueItem[];
  onClose: () => void;
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  onPlayFrom: (payload: { trackId?: number; path?: string }) => void;
}

export default function QueueModal({
  open,
  items,
  onClose,
  formatMs,
  placeholder,
  canPlay,
  onPlayFrom
}: QueueModalProps) {
  if (!open) return null;

  return (
    <div className="side-panel-backdrop" onClick={onClose}>
      <aside
        className="side-panel"
        aria-label="Queue"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="card-header">
          <span>Queue</span>
          <div className="card-actions">
            <span className="pill">{items.length} items</span>
            <button className="icon-btn small" onClick={onClose} aria-label="Close">
              <X className="icon" aria-hidden="true" />
            </button>
          </div>
        </div>
        <QueueList
          items={items}
          formatMs={formatMs}
          placeholder={placeholder}
          canPlay={canPlay}
          onPlayFrom={onPlayFrom}
        />
      </aside>
    </div>
  );
}
