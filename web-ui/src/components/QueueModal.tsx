import { QueueItem } from "../types";
import { useEffect, useRef, useState } from "react";
import QueueList from "./QueueList";
import Modal from "./Modal";
import { Trash2, X } from "lucide-react";

interface QueueModalProps {
  open: boolean;
  items: QueueItem[];
  onClose: () => void;
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  isPaused: boolean;
  onPause: () => void;
  onPlayFrom: (payload: { trackId?: number; path?: string }) => void;
  onClear: (clearQueue: boolean, clearHistory: boolean) => void;
}

export default function QueueModal({
  open,
  items,
  onClose,
  formatMs,
  placeholder,
  canPlay,
  isPaused,
  onPause,
  onPlayFrom,
  onClear
}: QueueModalProps) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [clearQueue, setClearQueue] = useState(true);
  const [clearHistory, setClearHistory] = useState(false);

  useEffect(() => {
    if (!open) return;
    if (!listRef.current) return;
    listRef.current.scrollTop = 0;
  }, [open, items]);

  if (!open) return null;

  return (
    <div
      className={`side-panel-backdrop${confirmOpen ? " confirm-open" : ""}`}
      onClick={onClose}
    >
      <aside
        className="side-panel queue-panel"
        aria-label="Queue"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="card-header">
          <span>Queue</span>
          <div className="card-actions">
            <span className="pill">{items.length} items</span>
            <button
              className="icon-btn small"
              onClick={() => {
                if (items.length === 0) return;
                setConfirmOpen(true);
              }}
              aria-label="Clear queue"
              title="Clear queue"
              disabled={items.length === 0}
            >
              <Trash2 className="icon" aria-hidden="true" />
            </button>
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
          isPaused={isPaused}
          onPause={onPause}
          listRef={listRef}
          onPlayFrom={onPlayFrom}
        />
      </aside>
      <Modal
        open={confirmOpen}
        title="Clear queue?"
        onClose={() => {
          setConfirmOpen(false);
          setClearQueue(true);
          setClearHistory(false);
        }}
      >
        <div className="modal-body">
          <div className="muted small">
            This will clear the upcoming queue.
          </div>
          <label className="modal-checkbox">
            <input
              type="checkbox"
              checked={clearQueue}
              onChange={(event) => setClearQueue(event.target.checked)}
            />
            <span>Clear queue</span>
          </label>
          <label className="modal-checkbox">
            <input
              type="checkbox"
              checked={clearHistory}
              onChange={(event) => setClearHistory(event.target.checked)}
            />
            <span>Clear history</span>
          </label>
          <div className="modal-actions">
            <button
              className="btn ghost small"
              type="button"
              onClick={() => {
                setConfirmOpen(false);
                setClearQueue(true);
                setClearHistory(false);
              }}
            >
              Cancel
            </button>
            <button
              className="btn small"
              type="button"
              onClick={() => {
                setConfirmOpen(false);
                onClear(clearQueue, clearHistory);
                setClearQueue(true);
                setClearHistory(false);
              }}
              disabled={!clearQueue && !clearHistory}
            >
              Clear
            </button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
