import { useEffect, useMemo, useState } from "react";
import { apiUrl, fetchJson, postJson } from "./api";

interface OutputInfo {
  id: string;
  name: string;
  kind: string;
  state: string;
  provider_name?: string | null;
  supported_rates?: { min_hz: number; max_hz: number } | null;
}

interface OutputsResponse {
  active_id: string | null;
  outputs: OutputInfo[];
}

interface QueueItemTrack {
  kind: "track";
  path: string;
  file_name: string;
  duration_ms?: number | null;
  sample_rate?: number | null;
  album?: string | null;
  artist?: string | null;
  format: string;
}

interface QueueItemMissing {
  kind: "missing";
  path: string;
}

type QueueItem = QueueItemTrack | QueueItemMissing;

interface QueueResponse {
  items: QueueItem[];
}

interface LibraryEntryDir {
  kind: "dir";
  path: string;
  name: string;
}

interface LibraryEntryTrack {
  kind: "track";
  path: string;
  file_name: string;
  duration_ms?: number | null;
  sample_rate?: number | null;
  album?: string | null;
  artist?: string | null;
  format: string;
}

type LibraryEntry = LibraryEntryDir | LibraryEntryTrack;

interface LibraryResponse {
  dir: string;
  entries: LibraryEntry[];
}

interface StatusResponse {
  now_playing?: string | null;
  paused?: boolean | null;
  elapsed_ms?: number | null;
  duration_ms?: number | null;
  source_codec?: string | null;
  source_bit_depth?: number | null;
  container?: string | null;
  output_sample_format?: string | null;
  resampling?: boolean | null;
  resample_from_hz?: number | null;
  resample_to_hz?: number | null;
  sample_rate?: number | null;
  output_sample_rate?: number | null;
  channels?: number | null;
  output_device?: string | null;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  format?: string | null;
  bitrate_kbps?: number | null;
  buffered_frames?: number | null;
  buffer_capacity_frames?: number | null;
}

function formatMs(ms?: number | null): string {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatHz(hz?: number | null): string {
  if (!hz) return "—";
  if (hz >= 1000) {
    return `${(hz / 1000).toFixed(1)} kHz`;
  }
  return `${hz} Hz`;
}

function formatRateRange(output: OutputInfo): string {
  if (!output.supported_rates) return "rate range unknown";
  return `${formatHz(output.supported_rates.min_hz)} - ${formatHz(output.supported_rates.max_hz)}`;
}

function parentDir(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed) return null;
  if (trimmed === "/") return null;
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return "/";
  return trimmed.slice(0, idx);
}

function sortLibraryEntries(entries: LibraryEntry[]): LibraryEntry[] {
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) {
      return a.kind === "dir" ? -1 : 1;
    }
    const aName = a.kind === "dir" ? a.name : a.file_name;
    const bName = b.kind === "dir" ? b.name : b.file_name;
    return aName.localeCompare(bName);
  });
}

export default function App() {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [activeOutputId, setActiveOutputId] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [libraryDir, setLibraryDir] = useState<string | null>(null);
  const [libraryEntries, setLibraryEntries] = useState<LibraryEntry[]>([]);
  const [libraryLoading, setLibraryLoading] = useState<boolean>(false);
  const [outputsOpen, setOutputsOpen] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);

  const activeOutput = useMemo(
    () => outputs.find((output) => output.id === activeOutputId) ?? null,
    [outputs, activeOutputId]
  );

  useEffect(() => {
    let mounted = true;
    async function loadOutputs() {
      try {
        const response = await fetchJson<OutputsResponse>("/outputs");
        if (!mounted) return;
        setOutputs(response.outputs);
        setActiveOutputId(response.active_id ?? null);
        setError(null);
      } catch (err) {
        if (!mounted) return;
        setError((err as Error).message);
      }
    }
    loadOutputs();
    if (!outputsOpen) {
      return () => {
        mounted = false;
      };
    }
    const timer = setInterval(loadOutputs, 5000);
    return () => {
      mounted = false;
      clearInterval(timer);
    };
  }, [outputsOpen]);

  useEffect(() => {
    if (!activeOutputId) {
      setStatus(null);
      return;
    }
    let mounted = true;
    const streamUrl = apiUrl(`/outputs/${encodeURIComponent(activeOutputId)}/status/stream`);

    const stream = new EventSource(streamUrl);
    stream.addEventListener("status", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as StatusResponse;
      setStatus(data);
      setUpdatedAt(new Date());
      setError(null);
    });
    stream.onerror = () => {
      if (!mounted) return;
      setError("Live status disconnected.");
    };

    return () => {
      mounted = false;
      stream.close();
    };
  }, [activeOutputId]);

  useEffect(() => {
    let mounted = true;
    const stream = new EventSource(apiUrl("/queue/stream"));
    stream.addEventListener("queue", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as QueueResponse;
      setQueue(data.items ?? []);
      setError(null);
    });
    stream.onerror = () => {
      if (!mounted) return;
      setError("Live queue disconnected.");
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, []);

  useEffect(() => {
    if (!outputsOpen) return;
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOutputsOpen(false);
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [outputsOpen]);

  useEffect(() => {
    let mounted = true;
    async function loadLibrary(dir?: string | null) {
      setLibraryLoading(true);
      try {
        const query = dir ? `?dir=${encodeURIComponent(dir)}` : "";
        const response = await fetchJson<LibraryResponse>(`/library${query}`);
        if (!mounted) return;
        setLibraryDir(response.dir);
        setLibraryEntries(sortLibraryEntries(response.entries));
        setError(null);
      } catch (err) {
        if (!mounted) return;
        setError((err as Error).message);
      } finally {
        if (mounted) setLibraryLoading(false);
      }
    }
    loadLibrary(libraryDir);
    return () => {
      mounted = false;
    };
  }, [libraryDir]);

  async function handlePause() {
    try {
      await postJson("/pause");
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handleNext() {
    try {
      await postJson("/queue/next");
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handleRescan() {
    try {
      await postJson("/library/rescan");
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handleSelectOutput(id: string) {
    try {
      await postJson("/outputs/select", { id });
      setActiveOutputId(id);
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handlePlay(path: string) {
    try {
      await postJson("/play", { path, queue_mode: "keep" });
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handleQueue(path: string) {
    try {
      await postJson("/queue", { paths: [path] });
    } catch (err) {
      setError((err as Error).message);
    }
  }

  return (
    <div className="app">
      <header className="hero">
        <div className="hero-left">
          <span className="eyebrow">Audio Hub</span>
          <h1>Lossless control with a live signal view.</h1>
          <p>
            A focused dashboard for your playback pipeline. Keep an eye on output state, signal
            metadata, and the queue without opening the TUI.
          </p>
          {error ? <div className="alert">{error}</div> : null}
        </div>
      </header>

      <section className="grid">
        <div className="card">
          <div className="card-header">
            <span>Library</span>
            <div className="card-actions">
              <span className="pill">{libraryEntries.length} items</span>
              <button className="btn ghost small" onClick={handleRescan}>
                Rescan
              </button>
            </div>
          </div>
          <div className="library-path">
            <span className="muted small">Path</span>
            <span className="mono">{libraryDir ?? "Loading..."}</span>
          </div>
          <div className="library-actions">
            <button
              className="btn ghost"
              disabled={!libraryDir || !parentDir(libraryDir)}
              onClick={() => {
                if (libraryDir) {
                  const parent = parentDir(libraryDir);
                  if (parent) setLibraryDir(parent);
                }
              }}
            >
              Up one level
            </button>
            <button
              className="btn ghost"
              onClick={() => setLibraryDir(null)}
              disabled={!libraryDir}
            >
              Back to root
            </button>
          </div>
          <div className="library-list">
            {libraryLoading ? <p className="muted">Loading library...</p> : null}
            {!libraryLoading &&
              libraryEntries.map((entry) => {
                if (entry.kind === "dir") {
                  return (
                    <button
                      key={entry.path}
                      className="library-row"
                      onClick={() => setLibraryDir(entry.path)}
                    >
                      <div>
                        <div className="library-title">{entry.name}</div>
                        <div className="muted small">Folder</div>
                      </div>
                      <span className="chip">Open</span>
                    </button>
                  );
                }
                return (
                  <div key={entry.path} className="library-row track">
                    <div>
                      <div className="library-title">{entry.file_name}</div>
                      <div className="muted small">
                        {entry.artist ?? "Unknown artist"}
                        {entry.album ? ` - ${entry.album}` : ""}
                      </div>
                    </div>
                    <div className="library-actions-inline">
                      <span className="muted small">{formatMs(entry.duration_ms)}</span>
                      <button className="btn ghost" onClick={() => handleQueue(entry.path)}>
                        Queue
                      </button>
                      <button className="btn" onClick={() => handlePlay(entry.path)}>
                        Play
                      </button>
                    </div>
                  </div>
                );
              })}
            {!libraryLoading && libraryEntries.length === 0 ? (
              <p className="muted">No entries found in this folder.</p>
            ) : null}
          </div>
        </div>

        <div className="card">
          <div className="card-header">
            <span>Queue</span>
            <span className="pill">{queue.length} items</span>
          </div>
          <div className="queue-list">
            {queue.map((item, index) => (
              <div key={`${item.kind}-${index}`} className="queue-row">
                {item.kind === "track" ? (
                  <>
                    <div>
                      <div className="queue-title">{item.file_name}</div>
                      <div className="muted small">
                        {item.artist ?? "Unknown artist"}
                        {item.album ? ` - ${item.album}` : ""}
                      </div>
                    </div>
                    <div className="queue-meta">
                      <span>{item.format}</span>
                      <span>{formatMs(item.duration_ms)}</span>
                    </div>
                  </>
                ) : (
                  <span className="muted">Missing: {item.path}</span>
                )}
              </div>
            ))}
            {queue.length === 0 ? (
              <p className="muted">Queue is empty. Add tracks from the TUI for now.</p>
            ) : null}
          </div>
        </div>

        <div className="card">
          <div className="card-header">
            <span>Signal</span>
            <span className="pill">{activeOutput?.name ?? "No output"}</span>
          </div>
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
        </div>
      </section>

      <div className="player-bar">
        <div className="player-left">
          <div className="album-art">Artwork</div>
          <div>
            <div className="track-title">{status?.title ?? status?.now_playing ?? "Idle"}</div>
            <div className="muted small">{status?.artist ?? "Unknown artist"}</div>
          </div>
        </div>
        <div className="player-middle">
          <div className="player-controls">
            <button className="icon-btn" aria-label="Previous" disabled>
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <rect x="3" y="5" width="2" height="14" rx="1" />
                <polygon points="21,5 13,12 21,19" />
                <polygon points="13,5 5,12 13,19" />
              </svg>
            </button>
            <button className="icon-btn primary" onClick={handlePause} aria-label="Play or pause">
              {status?.paused ? (
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <polygon points="7,5 19,12 7,19" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <rect x="6" y="5" width="4" height="14" rx="1" />
                  <rect x="14" y="5" width="4" height="14" rx="1" />
                </svg>
              )}
            </button>
            <button
              className="icon-btn"
              onClick={handleNext}
              aria-label="Next"
              disabled={queue.length === 0}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <rect x="19" y="5" width="2" height="14" rx="1" />
                <polygon points="3,5 11,12 3,19" />
                <polygon points="11,5 19,12 11,19" />
              </svg>
            </button>
          </div>
          <div className="progress">
            <div className="progress-track"></div>
            <div
              className="progress-fill"
              style={{
                width:
                  status?.duration_ms && status?.elapsed_ms
                    ? `${Math.min(100, (status.elapsed_ms / status.duration_ms) * 100)}%`
                    : "0%"
              }}
            ></div>
          </div>
          <div className="meta-row">
            <span>{formatMs(status?.elapsed_ms)} / {formatMs(status?.duration_ms)}</span>
            <span>{status?.format ?? "—"}</span>
          </div>
        </div>
        <div className="player-right">
          <div className="output-chip">
            <span className="muted small">Output</span>
            <span>{activeOutput?.name ?? "No output"}</span>
          </div>
          <button className="btn ghost small" onClick={() => setOutputsOpen(true)}>
            Select output
          </button>
        </div>
      </div>

      {outputsOpen ? (
        <div className="modal" onClick={() => setOutputsOpen(false)}>
          <div className="modal-card" onClick={(event) => event.stopPropagation()}>
            <div className="card-header">
              <span>Outputs</span>
              <div className="card-actions">
                <span className="pill">{outputs.length} devices</span>
                <button className="btn ghost small" onClick={() => setOutputsOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="output-list">
              {outputs.map((output) => (
                <button
                  key={output.id}
                  className={`output-row ${output.id === activeOutputId ? "active" : ""}`}
                  onClick={() => handleSelectOutput(output.id)}
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
          </div>
        </div>
      ) : null}
    </div>
  );
}
