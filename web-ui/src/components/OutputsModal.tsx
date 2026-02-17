import { OutputInfo } from "../types";
import Modal from "./Modal";

interface OutputsModalProps {
  open: boolean;
  outputs: OutputInfo[];
  activeOutputId: string | null;
  onClose: () => void;
  onSelectOutput: (id: string) => void;
  formatRateRange: (output: OutputInfo) => string;
}

export default function OutputsModal({
  open,
  outputs,
  activeOutputId,
  onClose,
  onSelectOutput,
  formatRateRange
}: OutputsModalProps) {
  return (
    <Modal
      open={open}
      title="Outputs"
      onClose={onClose}
      headerRight={<span className="pill">{outputs.length} devices</span>}
    >
      <div className="output-list-wrap">
        <div className="output-list">
          {outputs.map((output) => {
            const isActive = output.id === activeOutputId;
            const state = output.state?.toLowerCase() ?? "unknown";
            return (
              <button
                key={output.id}
                className={`output-row ${isActive ? "active" : ""}`}
                onClick={() => onSelectOutput(output.id)}
              >
                <div className="output-main">
                  <div className="output-title">{output.name}</div>
                  <div className="muted small">
                    {output.provider_name ?? output.kind} • {formatRateRange(output)}
                  </div>
                  <div className="muted small">{output.id}</div>
                </div>
                <div className="output-meta">
                  <span className={`output-status status-${state}`}>
                    <span className="status-dot" aria-hidden="true" />
                    {output.state}
                  </span>
                  <span className="chip">{isActive ? "active" : "select"}</span>
                </div>
              </button>
            );
          })}
          {outputs.length === 0 ? (
            <p className="muted">No outputs reported. Check provider discovery.</p>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}
