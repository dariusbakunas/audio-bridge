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
  const sourceCodec = status?.source_codec ?? status?.format ?? "Unknown source";
  const sourceDepth = status?.source_bit_depth ? `${status.source_bit_depth}-bit` : "bit depth —";
  const sourceRate = formatHz(status?.sample_rate);
  const outputRate = formatHz(status?.output_sample_rate);
  const outputFormat = status?.output_sample_format ?? "format —";
  const outputChannels = status?.channels ? `${status.channels}ch` : "channels —";
  const outputName = activeOutput?.name ?? "No output selected";
  const bridgeStage = status?.resampling ? "Decode + resample" : "Decode (direct rate)";
  const bridgeDetail = status?.resampling
    ? `${formatHz(status?.resample_from_hz ?? status?.sample_rate)} -> ${formatHz(
        status?.resample_to_hz ?? status?.output_sample_rate
      )}`
    : `${sourceRate} passthrough`;

  return (
    <Modal
      open={open}
      title="Signal"
      onClose={onClose}
      headerRight={<span className="pill">{activeOutput?.name ?? "No output"}</span>}
    >
      <div className="signal-flow" aria-label="Audio pipeline">
        <div className="signal-stage">
          <div className="signal-stage-title">Source</div>
          <div className="signal-stage-main">{sourceCodec}</div>
          <div className="signal-stage-sub">
            {sourceDepth} · {sourceRate}
          </div>
        </div>
        <div className={`signal-stage-arrow ${status?.resampling ? "resampling" : ""}`}>→</div>
        <div className="signal-stage">
          <div className="signal-stage-title">Bridge Processing</div>
          <div className="signal-stage-main">{bridgeStage}</div>
          <div className="signal-stage-sub">{bridgeDetail}</div>
        </div>
        <div className="signal-stage-arrow">→</div>
        <div className="signal-stage">
          <div className="signal-stage-title">Output</div>
          <div className="signal-stage-main">{outputName}</div>
          <div className="signal-stage-sub">
            {outputFormat} · {outputChannels} · {outputRate}
          </div>
        </div>
      </div>
      <div className="signal-grid">
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
