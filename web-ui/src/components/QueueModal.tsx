import { QueueItem } from "../types";
import QueueList from "./QueueList";

interface QueueModalProps {
  open: boolean;
  items: QueueItem[];
  onClose: () => void;
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
}

export default function QueueModal({
  open,
  items,
  onClose,
  formatMs,
  placeholder
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
            <button className="btn ghost small" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
        <QueueList items={items} formatMs={formatMs} placeholder={placeholder} />
      </aside>
    </div>
  );
}
