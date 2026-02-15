import { ReactNode, useEffect } from "react";
import { X } from "lucide-react";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  headerRight?: ReactNode;
  children: ReactNode;
}

export default function Modal({ open, title, onClose, headerRight, children }: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [open]);

  if (!open) return null;

  return (
    <div className="modal" onClick={onClose}>
      <div className="modal-card" onClick={(event) => event.stopPropagation()}>
        <div className="card-header">
          <span>{title}</span>
          <div className="card-actions">
            {headerRight}
            <button className="icon-btn small" onClick={onClose} aria-label="Close">
              <X className="icon" aria-hidden="true" />
            </button>
          </div>
        </div>
        {children}
      </div>
    </div>
  );
}
