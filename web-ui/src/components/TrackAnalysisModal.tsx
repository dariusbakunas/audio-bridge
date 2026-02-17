import { useEffect, useMemo, useRef, useState } from "react";
import { postJson } from "../api";
import { TrackAnalysisResponse } from "../types";
import Modal from "./Modal";

interface TrackAnalysisModalProps {
  open: boolean;
  trackId?: number | null;
  title: string;
  artist?: string | null;
  onClose: () => void;
}

function decodeBase64ToBytes(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function colorMap(value: number): [number, number, number] {
  const t = value / 255;
  const r = Math.min(255, Math.max(0, Math.floor(255 * Math.pow(t, 0.6))));
  const g = Math.min(255, Math.max(0, Math.floor(255 * Math.pow(t, 1.2))));
  const b = Math.min(255, Math.max(0, Math.floor(255 * Math.pow(t, 2.0))));
  return [r, g, b];
}

function formatDb(value?: number | null): string {
  if (value === null || value === undefined) return "—";
  return `${value.toFixed(1)} dB`;
}

function formatRatio(value?: number | null): string {
  if (value === null || value === undefined) return "—";
  const percent = value * 100;
  if (percent > 0 && percent < 0.01) return "<0.01%";
  if (percent < 0.1) return `${percent.toFixed(3)}%`;
  return `${percent.toFixed(2)}%`;
}

function formatHz(value?: number | null): string {
  if (!value) return "—";
  if (value >= 1000) return `${(value / 1000).toFixed(1)} kHz`;
  return `${value.toFixed(0)} Hz`;
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

export default function TrackAnalysisModal({
  open,
  trackId,
  title,
  artist,
  onClose
}: TrackAnalysisModalProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [analysis, setAnalysis] = useState<TrackAnalysisResponse | null>(null);
  const [duration] = useState(0);
  const [windowSize, setWindowSize] = useState(4096);
  const [cutoffHz, setCutoffHz] = useState(22_050);

  useEffect(() => {
    if (!open) return;
    if (!trackId) return;
    setLoading(true);
    setError(null);
    setAnalysis(null);
    postJson<TrackAnalysisResponse>("/tracks/analysis", {
      track_id: trackId,
      max_seconds: duration,
      width: 520,
      height: 160,
      window_size: windowSize,
      high_cutoff_hz: cutoffHz
    })
      .then((response) => {
        setAnalysis(response ?? null);
      })
      .catch((err) => {
        setError((err as Error).message);
      })
      .finally(() => {
        setLoading(false);
      });
  }, [open, trackId, duration, windowSize, cutoffHz]);

  const totalSeconds = useMemo(() => {
    if (analysis?.duration_ms) return analysis.duration_ms / 1000;
    if (duration > 0) return duration;
    return 0;
  }, [analysis?.duration_ms, duration]);
  const timeLabels = useMemo(() => {
    if (!analysis) return [] as Array<{ left: number; label: string }>;
    const steps = 5;
    return Array.from({ length: steps }, (_, idx) => {
      const ratio = idx / (steps - 1);
      return {
        left: ratio * 100,
        label: formatTime(totalSeconds * ratio)
      };
    });
  }, [analysis, totalSeconds]);
  const freqLabels = useMemo(() => {
    if (!analysis) return [] as Array<{ top: number; label: string }>;
    const nyquist = analysis.sample_rate / 2;
    return [0, 0.25, 0.5, 0.75, 1].map((ratio) => ({
      top: (1 - ratio) * 100,
      label: formatHz(nyquist * ratio)
    }));
  }, [analysis]);

  useEffect(() => {
    if (!analysis || !canvasRef.current) return;
    const canvas = canvasRef.current;
    canvas.width = analysis.width;
    canvas.height = analysis.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const bytes = decodeBase64ToBytes(analysis.data_base64);
    const image = ctx.createImageData(analysis.width, analysis.height);
    for (let i = 0; i < bytes.length; i += 1) {
      const [r, g, b] = colorMap(bytes[i]);
      const idx = i * 4;
      image.data[idx] = r;
      image.data[idx + 1] = g;
      image.data[idx + 2] = b;
      image.data[idx + 3] = 255;
    }
    ctx.putImageData(image, 0, 0);
  }, [analysis]);


  return (
    <Modal open={open} title="Track analysis" onClose={onClose} className="analysis-modal">
      <div className="analysis-modal-body">
        <div>
          <div className="analysis-title">{title || "Unknown track"}</div>
          <div className="analysis-artist">{artist || "Unknown artist"}</div>
        </div>
        {loading ? <p className="muted">Analyzing audio...</p> : null}
        {error ? <p className="muted">{error}</p> : null}
        {analysis ? (
          <div className="analysis-grid">
            <div className="analysis-spectrogram">
              <div className="analysis-spectrogram-grid">
                <div className="analysis-y-axis">
                  {freqLabels.map((label) => (
                    <div
                      key={`${label.label}-${label.top}`}
                      className="analysis-axis-label"
                      style={{ top: `${label.top}%` }}
                    >
                      {label.label}
                    </div>
                  ))}
                </div>
                <div className="analysis-spectrogram-canvas">
                  <canvas ref={canvasRef} />
                  <div
                    className="analysis-cutoff"
                    style={{
                      top: `${analysis ? (1 - cutoffHz / (analysis.sample_rate / 2)) * 100 : 0}%`
                    }}
                  />
                  <div
                    className="analysis-cutoff-label"
                    style={{
                      top: `${analysis ? (1 - cutoffHz / (analysis.sample_rate / 2)) * 100 : 0}%`
                    }}
                  >
                    {formatHz(cutoffHz)}
                  </div>
                  <div className="analysis-x-axis">
                    {timeLabels.map((label) => (
                      <span
                        key={`${label.label}-${label.left}`}
                        className="analysis-axis-label"
                        style={{
                          left: `${label.left}%`,
                          transform:
                            label.left <= 0
                              ? "translateX(0)"
                              : label.left >= 100
                                ? "translateX(-100%)"
                                : "translateX(-50%)"
                        }}
                      >
                        {label.label}
                      </span>
                    ))}
                  </div>
                </div>
              </div>
            </div>
            <div className="analysis-meta">
              <div className="analysis-meta-row">
                <span className="muted small">Sample rate</span>
                <span>{formatHz(analysis.sample_rate)}</span>
              </div>
            <div className="analysis-meta-row">
              <span className="muted small">95% rolloff</span>
              <span>{formatHz(analysis.heuristics?.rolloff_hz ?? null)}</span>
            </div>
              <div className="analysis-meta-row">
                <span className="muted small">
                  Ultrasonic &gt; {formatHz(cutoffHz)}
                  <span
                    className="analysis-help"
                    data-tooltip="Median per-frame ratio of energy above the cutoff to energy above 20 Hz. Near zero often indicates a brickwall filter or upsampled source."
                  >
                    ?
                  </span>
                </span>
                <span>{formatRatio(analysis.heuristics?.ultrasonic_ratio)}</span>
              </div>
              <div className="analysis-meta-row">
                <span className="muted small">
                  Upper audible (20-24k)
                  <span
                    className="analysis-help"
                    data-tooltip="Median per-frame ratio of energy between 20-24 kHz to energy above 20 Hz. Indicates presence of upper-audible content."
                  >
                    ?
                  </span>
                </span>
                <span>{formatRatio(analysis.heuristics?.upper_audible_ratio)}</span>
              </div>
              <div className="analysis-meta-row">
                <span className="muted small">Dynamic range</span>
                <span>{formatDb(analysis.heuristics?.dynamic_range_db ?? null)}</span>
              </div>
              <div className="analysis-meta-row">
                <span className="muted small">Window size</span>
                <span>{windowSize} samples</span>
              </div>
              <div className="analysis-meta-row">
                <span className="muted small">Cutoff</span>
                <span>{formatHz(cutoffHz)}</span>
              </div>
            </div>
          </div>
        ) : null}
        <div className="analysis-controls">
          <span className="muted small">FFT size</span>
          <div className="analysis-duration">
            {[2048, 4096, 8192].map((value) => (
              <button
                key={value}
                type="button"
                className={`btn ghost small${windowSize === value ? " active" : ""}`}
                onClick={() => setWindowSize(value)}
                disabled={loading}
              >
                {value}
              </button>
            ))}
          </div>
        </div>
        <div className="analysis-controls">
          <span className="muted small">Cutoff</span>
          <div className="analysis-duration">
            {[18000, 20000, 22050, 24000].map((value) => {
              const overNyquist =
                analysis?.sample_rate !== undefined
                  ? value > analysis.sample_rate / 2
                  : false;
              return (
                <button
                  key={value}
                  type="button"
                  className={`btn ghost small${cutoffHz === value ? " active" : ""}`}
                  onClick={() => setCutoffHz(value)}
                  disabled={loading || overNyquist}
                  title={overNyquist ? "Unavailable for this sample rate." : undefined}
                >
                  {value >= 1000 ? `${(value / 1000).toFixed(1)}kHz` : value}
                </button>
              );
            })}
          </div>
        </div>
        <div className="modal-actions">
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
