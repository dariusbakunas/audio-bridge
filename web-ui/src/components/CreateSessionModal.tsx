import Modal from "./Modal";

type CreateSessionModalProps = {
  open: boolean;
  busy: boolean;
  name: string;
  neverExpires: boolean;
  onNameChange: (value: string) => void;
  onNeverExpiresChange: (value: boolean) => void;
  onClose: () => void;
  onSubmit: () => void;
};

export default function CreateSessionModal({
  open,
  busy,
  name,
  neverExpires,
  onNameChange,
  onNeverExpiresChange,
  onClose,
  onSubmit
}: CreateSessionModalProps) {
  if (!open) return null;

  return (
    <Modal open={open} onClose={onClose} title="Create session">
      <div className="form-row">
        <label htmlFor="new-session-name">Name</label>
        <input
          id="new-session-name"
          className="input"
          value={name}
          onChange={(event) => onNameChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onSubmit();
            }
          }}
          disabled={busy}
          autoFocus
        />
      </div>
      <label className="checkbox" htmlFor="new-session-never-expires">
        <input
          id="new-session-never-expires"
          type="checkbox"
          checked={neverExpires}
          onChange={(event) => onNeverExpiresChange(event.target.checked)}
          disabled={busy}
        />
        <span>Never expires</span>
      </label>
      <div className="modal-actions">
        <button className="btn ghost" type="button" onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button
          className="btn"
          type="button"
          onClick={onSubmit}
          disabled={busy || name.trim().length === 0}
        >
          {busy ? "Creating..." : "Create"}
        </button>
      </div>
    </Modal>
  );
}
