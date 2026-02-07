import { useEffect, useMemo, useState, useCallback } from "react";
import { apiUrl, fetchJson, postJson } from "./api";
import {
  AlbumListResponse,
  AlbumSummary,
  LibraryEntry,
  LibraryResponse,
  MetadataEvent,
  OutputInfo,
  OutputsResponse,
  QueueItem,
  QueueResponse,
  TrackListResponse,
  TrackSummary
} from "./types";
import LibraryList from "./components/LibraryList";
import Modal from "./components/Modal";
import PlayerControls from "./components/PlayerControls";
import QueueList from "./components/QueueList";

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

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

const MAX_METADATA_EVENTS = 200;
const ALBUM_PLACEHOLDER = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='240' height='240'><rect width='100%25' height='100%25' fill='%23e9e4d8'/><rect x='12' y='12' width='216' height='216' rx='28' fill='%23fff9ef' stroke='%23d7cbb7' stroke-width='4'/><text x='50%25' y='54%25' font-family='Space Grotesk, sans-serif' font-size='24' fill='%239c7f63' text-anchor='middle'>No Art</text></svg>";

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

function describeMetadataEvent(event: MetadataEvent): { title: string; detail?: string } {
  switch (event.kind) {
    case "music_brainz_batch":
      return { title: "MusicBrainz batch", detail: `${event.count} candidates` };
    case "music_brainz_lookup_start":
      return {
        title: "MusicBrainz lookup started",
        detail: `${event.title} — ${event.artist}${event.album ? ` (${event.album})` : ""}`
      };
    case "music_brainz_lookup_success":
      return {
        title: "MusicBrainz lookup success",
        detail: event.recording_mbid ?? "match found"
      };
    case "music_brainz_lookup_no_match":
      return {
        title: "MusicBrainz lookup no match",
        detail: `${event.title} — ${event.artist}${event.album ? ` (${event.album})` : ""}`
      };
    case "music_brainz_lookup_failure":
      return { title: "MusicBrainz lookup failed", detail: event.error };
    case "cover_art_batch":
      return { title: "Cover art batch", detail: `${event.count} albums` };
    case "cover_art_fetch_start":
      return { title: "Cover art fetch started", detail: `album ${event.album_id}` };
    case "cover_art_fetch_success":
      return { title: "Cover art fetched", detail: `album ${event.album_id}` };
    case "cover_art_fetch_failure":
      return {
        title: "Cover art fetch failed",
        detail: `${event.error} (attempt ${event.attempts})`
      };
    default:
      return { title: "Metadata event" };
  }
}

function metadataDetailLines(event: MetadataEvent): string[] {
  if (event.kind !== "music_brainz_lookup_no_match") {
    if (event.kind === "cover_art_fetch_failure") {
      return [`MBID: ${event.mbid}`];
    }
    return [];
  }
  const lines: string[] = [];
  if (event.query) {
    lines.push(`Query: ${event.query}`);
  }
  if (event.top_score !== null && event.top_score !== undefined) {
    lines.push(`Top score: ${event.top_score}`);
  }
  if (event.best_recording_title || event.best_recording_id) {
    const title = event.best_recording_title ?? "unknown";
    const id = event.best_recording_id ? ` (${event.best_recording_id})` : "";
    lines.push(`Best: ${title}${id}`);
  }
  return lines;
}

export default function App() {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [activeOutputId, setActiveOutputId] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [libraryDir, setLibraryDir] = useState<string | null>(null);
  const [libraryEntries, setLibraryEntries] = useState<LibraryEntry[]>([]);
  const [libraryLoading, setLibraryLoading] = useState<boolean>(false);
  const [rescanBusy, setRescanBusy] = useState<boolean>(false);
  const [selectedTrackPath, setSelectedTrackPath] = useState<string | null>(null);
  const [trackMenuPath, setTrackMenuPath] = useState<string | null>(null);
  const [trackMenuPosition, setTrackMenuPosition] = useState<{
    top: number;
    right: number;
  } | null>(null);
  const [queueOpen, setQueueOpen] = useState<boolean>(false);
  const [signalOpen, setSignalOpen] = useState<boolean>(false);
  const [outputsOpen, setOutputsOpen] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [metadataEvents, setMetadataEvents] = useState<MetadataEventEntry[]>([]);
  const [albums, setAlbums] = useState<AlbumSummary[]>([]);
  const [albumsLoading, setAlbumsLoading] = useState<boolean>(false);
  const [albumsError, setAlbumsError] = useState<string | null>(null);
  const [albumViewId, setAlbumViewId] = useState<number | null>(null);
  const [albumTracks, setAlbumTracks] = useState<TrackSummary[]>([]);
  const [albumTracksLoading, setAlbumTracksLoading] = useState<boolean>(false);
  const [albumTracksError, setAlbumTracksError] = useState<string | null>(null);
  const [browserView, setBrowserView] = useState<"library" | "albums">("albums");
  const [nowPlayingCover, setNowPlayingCover] = useState<string | null>(null);
  const [nowPlayingCoverFailed, setNowPlayingCoverFailed] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);

  const activeOutput = useMemo(
    () => outputs.find((output) => output.id === activeOutputId) ?? null,
    [outputs, activeOutputId]
  );
  const canTogglePlayback = Boolean(
    activeOutputId && (status?.now_playing || selectedTrackPath)
  );
  const showPlayIcon = !status?.now_playing || Boolean(status?.paused);
  const isPlaying = Boolean(status?.now_playing && !status?.paused);
  const uiBuildId = useMemo(() => {
    if (__BUILD_MODE__ === "development") {
      return "dev";
    }
    return `v${__APP_VERSION__}+${__GIT_SHA__}`;
  }, []);
  const playButtonTitle = !activeOutputId
    ? "Select an output to control playback."
    : !status?.now_playing && !selectedTrackPath
      ? "Select a track to play."
    : undefined;
  const selectedAlbum = useMemo(
    () => albums.find((album) => album.id === albumViewId) ?? null,
    [albums, albumViewId]
  );

  useEffect(() => {
    let mounted = true;
    async function loadOutputs() {
      try {
        const response = await fetchJson<OutputsResponse>("/outputs");
        if (!mounted) return;
        const activeId = response.outputs.some((output) => output.id === response.active_id)
          ? response.active_id
          : null;
        setOutputs(response.outputs);
        setActiveOutputId(activeId);
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
    if (!trackMenuPath) return;
    function handleDocumentClick(event: MouseEvent) {
      const target = event.target as Element | null;
      if (target?.closest('[data-track-menu="true"]')) {
        return;
      }
      setTrackMenuPath(null);
      setTrackMenuPosition(null);
    }
    document.addEventListener("click", handleDocumentClick);
    return () => {
      document.removeEventListener("click", handleDocumentClick);
    };
  }, [trackMenuPath]);

  useEffect(() => {
    if (!isPlaying && signalOpen) {
      setSignalOpen(false);
    }
  }, [isPlaying, signalOpen]);

  useEffect(() => {
    let mounted = true;
    const stream = new EventSource(apiUrl("/outputs/stream"));
    stream.addEventListener("outputs", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as OutputsResponse;
      const activeId = data.outputs.some((output) => output.id === data.active_id)
        ? data.active_id
        : null;
      setOutputs(data.outputs);
      setActiveOutputId(activeId);
      setError(null);
    });
    stream.onerror = () => {
      if (!mounted) return;
      setError("Live outputs disconnected.");
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, []);

  async function handleRescanLibrary() {
    if (rescanBusy) return;
    setRescanBusy(true);
    try {
      await postJson("/library/rescan");
      setError(null);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setRescanBusy(false);
    }
  }

  async function handleRescanTrack(path: string) {
    if (rescanBusy) return;
    setRescanBusy(true);
    try {
      await postJson("/library/rescan/track", { path });
      setError(null);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setRescanBusy(false);
    }
  }

  useEffect(() => {
    if (!settingsOpen) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/metadata/stream"));
    stream.addEventListener("metadata", (event) => {
      if (!mounted) return;
      const data = JSON.parse((event as MessageEvent).data) as MetadataEvent;
      setMetadataEvents((prev) => {
        const entry: MetadataEventEntry = {
          id: Date.now() + Math.floor(Math.random() * 1000),
          time: new Date(),
          event: data
        };
        return [entry, ...prev].slice(0, MAX_METADATA_EVENTS);
      });
    });
    stream.onerror = () => {
      if (!mounted) return;
      setError("Live metadata updates disconnected.");
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [settingsOpen]);

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
    const path = status?.now_playing ?? null;
    if (!path) {
      setNowPlayingCover(null);
      setNowPlayingCoverFailed(false);
      return;
    }
    setNowPlayingCover(apiUrl(`/art?path=${encodeURIComponent(path)}`));
    setNowPlayingCoverFailed(false);
  }, [status?.now_playing]);

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
        setSelectedTrackPath(null);
        setTrackMenuPath(null);
        setTrackMenuPosition(null);
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

  const loadAlbums = useCallback(async () => {
    setAlbumsLoading(true);
    try {
      const response = await fetchJson<AlbumListResponse>("/albums?limit=200");
      setAlbums(response.items ?? []);
      setAlbumsError(null);
    } catch (err) {
      setAlbumsError((err as Error).message);
    } finally {
      setAlbumsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadAlbums();
  }, [loadAlbums]);

  useEffect(() => {
    let mounted = true;
    const stream = new EventSource(apiUrl("/albums/stream"));
    stream.addEventListener("albums", () => {
      if (!mounted) return;
      loadAlbums();
    });
    stream.onerror = () => {
      if (!mounted) return;
      setAlbumsError("Live albums disconnected.");
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [loadAlbums]);

  useEffect(() => {
    if (albumViewId === null) return;
    let mounted = true;
    async function loadAlbumTracks() {
      setAlbumTracksLoading(true);
      try {
        const response = await fetchJson<TrackListResponse>(
          `/tracks?album_id=${albumViewId}&limit=500`
        );
        if (!mounted) return;
        setAlbumTracks(response.items ?? []);
        setAlbumTracksError(null);
      } catch (err) {
        if (!mounted) return;
        setAlbumTracksError((err as Error).message);
      } finally {
        if (mounted) setAlbumTracksLoading(false);
      }
    }
    loadAlbumTracks();
    return () => {
      mounted = false;
    };
  }, [albumViewId]);

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

  async function handlePlayAlbumTrack(track: TrackSummary) {
    if (!track.path) return;
    await handlePlay(track.path);
  }

  async function handleQueueAlbumTrack(track: TrackSummary) {
    if (!track.path) return;
    await handleQueue(track.path);
  }

  async function handlePlayAlbum() {
    if (!albumTracks.length) return;
    const paths = albumTracks.map((track) => track.path).filter(Boolean);
    if (!paths.length) return;
    const [first, ...rest] = paths;
    try {
      await postJson("/queue/clear");
      if (rest.length > 0) {
        await postJson("/queue", { paths: rest });
      }
      await postJson("/play", { path: first, queue_mode: "keep" });
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

  async function handlePlayNext(path: string) {
    try {
      await postJson("/queue/next/add", { paths: [path] });
    } catch (err) {
      setError((err as Error).message);
    }
  }

  async function handlePrimaryAction() {
    if (status?.now_playing) {
      await handlePause();
      return;
    }
    if (selectedTrackPath) {
      await handlePlay(selectedTrackPath);
    }
  }

  return (
    <div className={`app ${settingsOpen ? "settings-mode" : ""}`}>
      <div className="layout">
        <aside className="side-nav">
          <div className="nav-brand">
            <span className="eyebrow">Audio Hub</span>
            <button
              className="icon-btn settings-btn"
              onClick={() => setSettingsOpen(!settingsOpen)}
              aria-label="Settings"
              title="Settings"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path
                  d="M12 8.75a3.25 3.25 0 1 0 0 6.5 3.25 3.25 0 0 0 0-6.5Zm9.25 3.25c0-.5-.03-1-.1-1.48l2.02-1.57a.75.75 0 0 0 .17-.96l-1.92-3.32a.75.75 0 0 0-.91-.34l-2.38.96a9.5 9.5 0 0 0-2.56-1.48l-.36-2.52A.75.75 0 0 0 14.41 1h-3.82a.75.75 0 0 0-.74.64l-.36 2.52a9.5 9.5 0 0 0-2.56 1.48l-2.38-.96a.75.75 0 0 0-.91.34L1.72 8.34a.75.75 0 0 0 .17.96l2.02 1.57c-.07.48-.1.98-.1 1.48s.03 1 .1 1.48l-2.02 1.57a.75.75 0 0 0-.17.96l1.92 3.32a.75.75 0 0 0 .91.34l2.38-.96a9.5 9.5 0 0 0 2.56 1.48l.36 2.52a.75.75 0 0 0 .74.64h3.82a.75.75 0 0 0 .74-.64l.36-2.52a9.5 9.5 0 0 0 2.56-1.48l2.38.96a.75.75 0 0 0 .91-.34l1.92-3.32a.75.75 0 0 0-.17-.96l-2.02-1.57c.07-.48.1-.98.1-1.48Z"
                  fill="currentColor"
                />
              </svg>
            </button>
          </div>
          <div className="nav-section">
            <div className="nav-label">Library</div>
            <button
              className={`nav-button ${browserView === "albums" ? "active" : ""}`}
              onClick={() => {
                setAlbumViewId(null);
                setBrowserView("albums");
                setSettingsOpen(false);
              }}
            >
              Albums
            </button>
            <button
              className={`nav-button ${browserView === "library" ? "active" : ""}`}
              onClick={() => {
                setAlbumViewId(null);
                setBrowserView("library");
                setSettingsOpen(false);
              }}
            >
              Library
            </button>
          </div>
        </aside>

        <main className="main">
          <header className={`hero ${settingsOpen ? "hidden" : ""}`}>
            <h1>Lossless control with a live signal view.</h1>
            <p>
              A focused dashboard for your playback pipeline. Keep an eye on output state, signal
              metadata, and the queue without opening the TUI.
            </p>
            {error ? <div className="alert">{error}</div> : null}
          </header>

          {!settingsOpen && albumViewId === null ? (
            <section className="grid">
              {browserView === "albums" ? (
                <div className="card">
                  <div className="card-header">
                    <span>Albums</span>
                    <div className="card-actions">
                      <span className="pill">{albums.length} albums</span>
                    </div>
                  </div>
                  {albumsLoading ? <p className="muted">Loading albums...</p> : null}
                  {albumsError ? <p className="muted">{albumsError}</p> : null}
                  {!albumsLoading && !albumsError ? (
                    <div className="album-grid">
                      {albums.map((album) => (
                        <button
                          key={album.id}
                          className="album-card"
                          onClick={() => setAlbumViewId(album.id)}
                        >
                          <img
                            className="album-cover"
                            src={album.cover_art_url ?? ALBUM_PLACEHOLDER}
                            alt={album.title}
                            loading="lazy"
                          />
                          <div className="album-card-info">
                            <div className="album-title">{album.title}</div>
                            <div className="muted small">{album.artist ?? "Unknown artist"}</div>
                          </div>
                        </button>
                      ))}
                      {albums.length === 0 ? <p className="muted">No albums found.</p> : null}
                    </div>
                  ) : null}
                </div>
              ) : null}

              {browserView === "library" ? (
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
                  <LibraryList
                    entries={libraryEntries}
                    loading={libraryLoading}
                    selectedTrackPath={selectedTrackPath}
                    trackMenuPath={trackMenuPath}
                    trackMenuPosition={trackMenuPosition}
                    canPlay={Boolean(activeOutputId)}
                    formatMs={formatMs}
                    onSelectDir={setLibraryDir}
                    onSelectTrack={setSelectedTrackPath}
                    onToggleMenu={(path, target) => {
                      if (trackMenuPath === path) {
                        setTrackMenuPath(null);
                        setTrackMenuPosition(null);
                        return;
                      }
                      const rect = target.getBoundingClientRect();
                      setTrackMenuPosition({
                        top: rect.bottom + 6,
                        right: window.innerWidth - rect.right
                      });
                      setTrackMenuPath(path);
                    }}
                    onPlay={(path) => {
                      handlePlay(path);
                      setTrackMenuPath(null);
                      setTrackMenuPosition(null);
                    }}
                    onQueue={(path) => {
                      handleQueue(path);
                      setTrackMenuPath(null);
                      setTrackMenuPosition(null);
                    }}
                    onPlayNext={(path) => {
                      handlePlayNext(path);
                      setTrackMenuPath(null);
                      setTrackMenuPosition(null);
                    }}
                    onRescan={(path) => {
                      handleRescanTrack(path);
                      setTrackMenuPath(null);
                      setTrackMenuPosition(null);
                    }}
                  />
                </div>
              ) : null}
            </section>
          ) : null}

          {albumViewId !== null && !settingsOpen ? (
            <section className="album-view">
              <div className="album-header">
                <button className="btn ghost small" onClick={() => setAlbumViewId(null)}>
                  Back to albums
                </button>
              </div>
              <div className="card album-detail">
                <div className="album-detail-top">
                  <div className="album-detail-left">
                    <img
                      className="album-cover large"
                      src={selectedAlbum?.cover_art_url ?? ALBUM_PLACEHOLDER}
                      alt={selectedAlbum?.title ?? "Album art"}
                    />
                  </div>
                  <div className="album-detail-right">
                    <div className="album-meta">
                      <div className="eyebrow">Album</div>
                      <h2>{selectedAlbum?.title ?? "Unknown album"}</h2>
                      <div className="muted">{selectedAlbum?.artist ?? "Unknown artist"}</div>
                      <div className="muted small">
                        {selectedAlbum?.year ? `${selectedAlbum.year} · ` : ""}
                        {selectedAlbum?.track_count ?? albumTracks.length} tracks
                      </div>
                      <div className="muted small">
                        {selectedAlbum?.mbid ? `MBID: ${selectedAlbum.mbid}` : "MBID: —"}
                      </div>
                      <div className="muted small">
                        {selectedAlbum?.cover_art_url
                          ? "Cover: cached"
                          : selectedAlbum?.mbid
                            ? "Cover: not cached"
                            : "Cover: unavailable"}
                      </div>
                      <button
                        className="btn ghost small"
                        onClick={handlePlayAlbum}
                        disabled={!activeOutputId || albumTracks.length === 0}
                      >
                        Play album
                      </button>
                    </div>
                  </div>
                </div>
                <div className="album-tracklist">
                  {albumTracksLoading ? <p className="muted">Loading tracks...</p> : null}
                  {albumTracksError ? <p className="muted">{albumTracksError}</p> : null}
                  {!albumTracksLoading && !albumTracksError ? (
                    <div className="album-tracks">
                      {albumTracks.map((track) => (
                        <div key={track.id} className="album-track-row">
                          <div>
                            <div className="album-track-title">
                              {track.track_number ? `${track.track_number}. ` : ""}
                              {track.title ?? track.file_name}
                            </div>
                            <div className="muted small">{track.artist ?? "Unknown artist"}</div>
                          </div>
                          <div className="album-track-actions">
                            <span className="muted small">{formatMs(track.duration_ms)}</span>
                            <button
                              className="btn ghost small"
                              onClick={() => handlePlayAlbumTrack(track)}
                              disabled={!activeOutputId}
                            >
                              Play
                            </button>
                            <button
                              className="btn ghost small"
                              onClick={() => handleQueueAlbumTrack(track)}
                            >
                              Queue
                            </button>
                          </div>
                        </div>
                      ))}
                      {albumTracks.length === 0 ? (
                        <div className="muted small">No tracks found for this album.</div>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              </div>
            </section>
          ) : null}

          <section className={`settings-screen ${settingsOpen ? "active" : ""}`}>
            <div className="card">
              <div className="card-header">
                <span>Metadata jobs</span>
                <div className="card-actions">
                  <button className="btn ghost small" onClick={() => setMetadataEvents([])}>
                    Clear
                  </button>
                  <span className="pill">{metadataEvents.length} events</span>
                </div>
              </div>
              <div className="settings-panel">
                <div className="muted small">Live MusicBrainz and cover art updates.</div>
                <div className="settings-actions">
                  <button
                    className="btn ghost small"
                    onClick={handleRescanLibrary}
                    disabled={rescanBusy}
                  >
                    {rescanBusy ? "Rescanning..." : "Rescan library"}
                  </button>
                </div>
                <div className="settings-list">
                  {metadataEvents.map((entry) => {
                    const info = describeMetadataEvent(entry.event);
                    const extraLines = metadataDetailLines(entry.event);
                    return (
                      <div key={entry.id} className="settings-row">
                        <div>
                          <div className="settings-title">{info.title}</div>
                          <div className="muted small">{info.detail ?? "—"}</div>
                          {extraLines.map((line) => (
                            <div key={line} className="muted small">
                              {line}
                            </div>
                          ))}
                        </div>
                        <div className="muted small">{entry.time.toLocaleTimeString()}</div>
                      </div>
                    );
                  })}
                  {metadataEvents.length === 0 ? (
                    <div className="muted small">No metadata events yet.</div>
                  ) : null}
                </div>
              </div>
            </div>
          </section>
        </main>
      </div>

      <div className={`player-bar ${settingsOpen ? "hidden" : ""}`}>
        <div className="player-left">
          {status?.title || status?.now_playing ? (
            <div className="album-art">
              {nowPlayingCover && !nowPlayingCoverFailed ? (
                <img
                  className="album-art-image"
                  src={nowPlayingCover}
                  alt={status?.album ?? status?.title ?? "Album art"}
                  onError={() => setNowPlayingCoverFailed(true)}
                />
              ) : (
                <span>Artwork</span>
              )}
            </div>
          ) : null}
          <div>
            <div className="track-title">
              {status?.title ?? status?.now_playing ?? "Nothing playing"}
            </div>
            <div className="muted small">
              {status?.artist ?? (status?.now_playing ? "Unknown artist" : "Select a track to start")}
            </div>
          </div>
        </div>
        <div className="player-middle">
          <PlayerControls
            isPlaying={isPlaying}
            canTogglePlayback={canTogglePlayback}
            showPlayIcon={showPlayIcon}
            playButtonTitle={playButtonTitle}
            queueHasItems={queue.length > 0}
            onPrimaryAction={handlePrimaryAction}
            onNext={handleNext}
            onSignalOpen={() => setSignalOpen(true)}
            onQueueOpen={() => setQueueOpen(true)}
          />
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
          <div className="muted small build-footer">UI build: {uiBuildId}</div>
        </div>
      </div>

      <Modal
        open={outputsOpen}
        title="Outputs"
        onClose={() => setOutputsOpen(false)}
        headerRight={<span className="pill">{outputs.length} devices</span>}
      >
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
      </Modal>

      <Modal
        open={signalOpen}
        title="Signal"
        onClose={() => setSignalOpen(false)}
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

      <Modal
        open={queueOpen}
        title="Queue"
        onClose={() => setQueueOpen(false)}
        headerRight={<span className="pill">{queue.length} items</span>}
      >
        <QueueList items={queue} formatMs={formatMs} />
      </Modal>

    </div>
  );
}
