import { useEffect, useRef, useState } from "react";
import { Trash2 } from "lucide-react";

import { QueueItem } from "../types";
import Modal from "./Modal";
import QueueList from "./QueueList";

type QueueScreenProps = {
  items: QueueItem[];
  formatMs: (ms?: number | null) => string;
  placeholder: (title?: string | null, artist?: string | null) => string;
  canPlay: boolean;
  isPaused: boolean;
  onPause: () => void | Promise<void>;
  onPlayFrom: (trackId: number) => void | Promise<void>;
  onClear: (clearQueue: boolean, clearHistory: boolean) => void | Promise<void>;
};

export default function QueueScreen({
  items,
  formatMs,
  placeholder,
  canPlay,
  isPaused,
  onPause,
  onPlayFrom,
  onClear
}: QueueScreenProps) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [clearQueue, setClearQueue] = useState(true);
  const [clearHistory, setClearHistory] = useState(false);

  useEffect(() => {
    if (!listRef.current) return;
    listRef.current.scrollTop = 0;
  }, [items]);

  return (
    <section className="queue-screen">
      <div className="card queue-screen-card">
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
              type="button"
            >
              <Trash2 className="icon" aria-hidden="true" />
            </button>
          </div>
        </div>
        <QueueList
          items={items}
          formatMs={formatMs}
          placeholder={placeholder}
          canPlay={canPlay}
          isPaused={isPaused}
          onPause={() => {
            void onPause();
          }}
          listRef={listRef}
          onPlayFrom={(trackId) => {
            void onPlayFrom(trackId);
          }}
        />
      </div>
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
                void onClear(clearQueue, clearHistory);
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
    </section>
  );
}
