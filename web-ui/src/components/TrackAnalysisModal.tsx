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
  const t = Math.min(1, Math.max(0, value / 255));
  const curve = Math.pow(t, 0.9);
  const stops: Array<[number, [number, number, number]]> = [
    [0.0, [2, 6, 20]],    // near-black
    [0.2, [12, 20, 90]],  // deep blue
    [0.4, [80, 30, 140]], // purple
    [0.6, [200, 40, 40]], // red
    [0.8, [255, 140, 0]], // orange
    [0.92, [255, 210, 40]], // yellow
    [1.0, [255, 245, 230]]  // near-white
  ];
  const blend = (a: number, b: number, p: number) => Math.round(a + (b - a) * p);
  for (let i = 1; i < stops.length; i += 1) {
    const [t1, c1] = stops[i];
    const [t0, c0] = stops[i - 1];
    if (curve <= t1) {
      const p = (curve - t0) / (t1 - t0);
      return [
        blend(c0[0], c1[0], p),
        blend(c0[1], c1[1], p),
        blend(c0[2], c1[2], p)
      ];
    }
  }
  return stops[stops.length - 1][1];
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
  if (value === null || value === undefined) return "—";
  if (value <= 0) return "0 Hz";
  if (value >= 1000) return `${(value / 1000).toFixed(1)} kHz`;
  return `${value.toFixed(0)} Hz`;
}

function openSpectrogramTab(
  canvas: HTMLCanvasElement | null,
  title: string,
  artist?: string | null,
  sampleRate?: number | null,
  durationSeconds?: number
) {
  if (!canvas) return;
  const scale = 2;
  const exportCanvas = document.createElement("canvas");
  exportCanvas.width = canvas.width * scale;
  exportCanvas.height = canvas.height * scale;
  const exportCtx = exportCanvas.getContext("2d");
  if (!exportCtx) return;
  exportCtx.imageSmoothingEnabled = false;
  exportCtx.drawImage(canvas, 0, 0, exportCanvas.width, exportCanvas.height);
  const imageUrl = exportCanvas.toDataURL("image/png");
  const safeTitle = title || "Track analysis";
  const safeArtist = artist || "";
  const tab = window.open("", "_blank");
  if (!tab) return;
  const nyquist = sampleRate ? sampleRate / 2 : null;
  const freqLabels = nyquist
    ? [0.25, 0.5, 0.75, 1].map((ratio) => ({
        top: (1 - ratio) * 100,
        label: formatHz(nyquist * ratio)
      }))
    : [];
  const timeLabels = durationSeconds
    ? Array.from({ length: 5 }, (_, idx) => {
        const ratio = idx / 4;
        return { left: ratio * 100, label: formatTime(durationSeconds * ratio) };
      })
    : [];

  tab.document.write(`<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${safeTitle}</title>
    <style>
      :root {
        color-scheme: dark;
      }
      body {
        margin: 0;
        font-family: "Space Grotesk", sans-serif;
        background: #0f1215;
        color: #f3f4f6;
      }
      .page {
        padding: 24px;
        max-width: 1200px;
        margin: 0 auto;
      }
      h1 {
        margin: 0 0 4px;
        font-size: 1.4rem;
      }
      .artist {
        color: #b8c0cc;
        margin-bottom: 16px;
      }
      .spectrogram-wrap {
        display: grid;
        grid-template-columns: 70px 1fr 70px;
        gap: 12px;
        align-items: stretch;
      }
      .axis-y {
        position: relative;
        font-size: 0.65rem;
        color: #b8c0cc;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }
      .axis-y span {
        position: absolute;
        right: 0;
        transform: translateY(-50%);
        white-space: nowrap;
      }
      .spectrogram {
        position: relative;
      }
      img {
        width: 100%;
        height: auto;
        border: 1px solid rgba(255, 255, 255, 0.1);
        border-radius: 6px;
        background: #0b0e12;
      }
      .axis-x {
        position: relative;
        height: 16px;
        margin-top: 6px;
        font-size: 0.65rem;
        color: #b8c0cc;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }
      .axis-x span {
        position: absolute;
        top: 0;
        transform: translateX(-50%);
        white-space: nowrap;
      }
      .legend {
        position: relative;
        display: grid;
        grid-template-columns: 12px 1fr;
        gap: 6px;
      }
      .bar {
        border-radius: 6px;
        border: 1px solid rgba(255, 255, 255, 0.1);
        background: linear-gradient(
          to bottom,
          rgb(255, 245, 230) 0%,
          rgb(255, 210, 40) 10%,
          rgb(255, 140, 0) 20%,
          rgb(200, 40, 40) 35%,
          rgb(80, 30, 140) 55%,
          rgb(12, 20, 90) 75%,
          rgb(2, 6, 20) 100%
        );
      }
      .labels {
        position: relative;
        font-size: 0.65rem;
        color: #b8c0cc;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }
      .labels span {
        position: absolute;
        right: 0;
        transform: translateY(-50%);
        white-space: nowrap;
      }
    </style>
  </head>
  <body>
    <div class="page">
      <h1>${safeTitle}</h1>
      ${safeArtist ? `<div class="artist">${safeArtist}</div>` : ""}
      <div class="spectrogram-wrap">
        <div class="axis-y">
          ${freqLabels
            .map((label) => `<span style="top:${label.top}%">${label.label}</span>`)
            .join("")}
        </div>
        <div class="spectrogram">
          <img src="${imageUrl}" alt="Spectrogram" />
          <div class="axis-x">
            ${timeLabels
              .map((label) => `<span style="left:${label.left}%">${label.label}</span>`)
              .join("")}
          </div>
        </div>
        <div class="legend">
          <div class="bar"></div>
          <div class="labels">
            ${[
              "0 dB",
              "-10 dB",
              "-20 dB",
              "-30 dB",
              "-40 dB",
              "-50 dB",
              "-60 dB",
              "-70 dB",
              "-80 dB",
              "-90 dB",
              "-100 dB",
              "-110 dB",
              "-120 dB"
            ]
              .map((label, index, arr) => {
                const top = (index / (arr.length - 1)) * 100;
                return `<span style="top:${top}%">${label}</span>`;
              })
              .join("")}
          </div>
        </div>
      </div>
    </div>
  </body>
</html>`);
  tab.document.close();
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
    return [0.25, 0.5, 0.75, 1].map((ratio) => ({
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
                <div className="analysis-spectrogram-wrap">
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
                  <div className="analysis-color-legend">
                    <div className="analysis-color-bar" />
                    <div className="analysis-color-labels">
                      {[
                        { label: "0 dB", top: 0 },
                        { label: "-10 dB", top: 10 },
                        { label: "-20 dB", top: 20 },
                        { label: "-30 dB", top: 30 },
                        { label: "-40 dB", top: 40 },
                        { label: "-50 dB", top: 50 },
                        { label: "-60 dB", top: 60 },
                        { label: "-70 dB", top: 70 },
                        { label: "-80 dB", top: 80 },
                        { label: "-90 dB", top: 90 },
                        { label: "-100 dB", top: 100 },
                        { label: "-110 dB", top: 110 },
                        { label: "-120 dB", top: 120 }
                      ].map((item) => (
                        <span
                          key={item.label}
                          className="analysis-color-label"
                          style={{ top: `${item.top / 120 * 100}%` }}
                        >
                          {item.label}
                        </span>
                      ))}
                    </div>
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
          <span className="muted small">
            FFT size
            <span
              className="analysis-help"
              data-tooltip="Controls time vs frequency detail. Larger sizes sharpen frequency resolution but smear time detail."
            >
              ?
            </span>
          </span>
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
          <button
            className="btn ghost"
            type="button"
            onClick={() =>
              openSpectrogramTab(
                canvasRef.current,
                title,
                artist,
                analysis?.sample_rate ?? null,
                analysis?.duration_ms ? analysis.duration_ms / 1000 : undefined
              )
            }
            disabled={!analysis}
          >
            Open full view
          </button>
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
