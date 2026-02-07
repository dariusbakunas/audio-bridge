import { OutputInfo, StatusResponse } from "../types";
import Modal from "./Modal";

interface SignalModalProps {
  open: boolean;
  status: StatusResponse | null;
  activeOutput: OutputInfo | null;
  updatedAt: Date | null;
  formatHz: (hz?: number | null) => string;
  onClose: () => void;
}

export default function SignalModal({
  open,
  status,
  activeOutput,
  updatedAt,
  formatHz,
  onClose
}: SignalModalProps) {
  return (
    <Modal
      open={open}
      title="Signal"
      onClose={onClose}
      headerRight={<span className="pill">{activeOutput?.name ?? "No output"}</span>}
    >
      <div className="signal-grid">
        <div>
          <div className="signal-label">Source</div>
          <div className="signal-value">
            {status?.source_codec ?? status?.format ?? "—"}
            {status?.source_bit_depth ? ` - ${status.source_bit_depth}-bit` : ""}
          </div>
        </div>
        <div>
          <div className="signal-label">Sample rate</div>
          <div className="signal-value">{formatHz(status?.sample_rate)}</div>
        </div>
        <div>
          <div className="signal-label">Output rate</div>
          <div className="signal-value">{formatHz(status?.output_sample_rate)}</div>
        </div>
        <div>
          <div className="signal-label">Resample</div>
          <div className="signal-value">
            {status?.resampling ? "Enabled" : "Direct"}
            {status?.resample_to_hz ? ` → ${formatHz(status.resample_to_hz)}` : ""}
          </div>
        </div>
        <div>
          <div className="signal-label">Output format</div>
          <div className="signal-value">{status?.output_sample_format ?? "—"}</div>
        </div>
        <div>
          <div className="signal-label">Channels</div>
          <div className="signal-value">{status?.channels ?? "—"}</div>
        </div>
        <div>
          <div className="signal-label">Bitrate</div>
          <div className="signal-value">
            {status?.bitrate_kbps ? `${status.bitrate_kbps} kbps` : "—"}
          </div>
        </div>
        <div>
          <div className="signal-label">Buffer</div>
          <div className="signal-value">
            {status?.buffered_frames && status?.buffer_capacity_frames
              ? `${status.buffered_frames} / ${status.buffer_capacity_frames} frames`
              : "—"}
          </div>
        </div>
      </div>
      <div className="muted small updated">
        Updated {updatedAt ? updatedAt.toLocaleTimeString() : "—"}
      </div>
    </Modal>
  );
}
