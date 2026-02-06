import { ReactNode } from "react";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  headerRight?: ReactNode;
  children: ReactNode;
}

export default function Modal({ open, title, onClose, headerRight, children }: ModalProps) {
  if (!open) return null;

  return (
    <div className="modal" onClick={onClose}>
      <div className="modal-card" onClick={(event) => event.stopPropagation()}>
        <div className="card-header">
          <span>{title}</span>
          <div className="card-actions">
            {headerRight}
            <button className="btn ghost small" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
        {children}
      </div>
    </div>
  );
}
