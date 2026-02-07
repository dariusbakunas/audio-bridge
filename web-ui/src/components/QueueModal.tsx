import { QueueItem } from "../types";
import Modal from "./Modal";
import QueueList from "./QueueList";

interface QueueModalProps {
  open: boolean;
  items: QueueItem[];
  onClose: () => void;
  formatMs: (ms?: number | null) => string;
}

export default function QueueModal({ open, items, onClose, formatMs }: QueueModalProps) {
  return (
    <Modal
      open={open}
      title="Queue"
      onClose={onClose}
      headerRight={<span className="pill">{items.length} items</span>}
    >
      <QueueList items={items} formatMs={formatMs} />
    </Modal>
  );
}
