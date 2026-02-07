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
      <div className="output-list">
        {outputs.map((output) => (
          <button
            key={output.id}
            className={`output-row ${output.id === activeOutputId ? "active" : ""}`}
            onClick={() => onSelectOutput(output.id)}
          >
            <div>
              <div className="output-title">{output.name}</div>
              <div className="muted small">
                {output.provider_name ?? output.kind} - {output.state} - {formatRateRange(output)}
              </div>
            </div>
            <span className="chip">{output.id === activeOutputId ? "active" : "select"}</span>
          </button>
        ))}
        {outputs.length === 0 ? (
          <p className="muted">No outputs reported. Check provider discovery.</p>
        ) : null}
      </div>
    </Modal>
  );
}
