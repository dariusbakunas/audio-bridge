import Modal from "./Modal";

interface AlbumNotesModalProps {
  open: boolean;
  title: string;
  artist: string;
  notes: string;
  onClose: () => void;
}

export default function AlbumNotesModal({
  open,
  title,
  artist,
  notes,
  onClose
}: AlbumNotesModalProps) {
  return (
    <Modal open={open} title="Album notes" onClose={onClose}>
      <div className="album-notes-modal">
        <div className="album-notes-modal-title">{title || "Unknown album"}</div>
        <div className="album-notes-modal-artist">{artist || "Unknown artist"}</div>
        <div className="album-notes-modal-text">{notes}</div>
        <div className="modal-actions">
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
