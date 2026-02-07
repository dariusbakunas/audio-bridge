import { useEffect, useMemo, useState, useCallback, useRef, SetStateAction} from "react";
import {apiUrl, fetchJson, postJson} from "./api";
import {
  AlbumListResponse,
  AlbumSummary,
  LibraryEntry,
  LibraryResponse,
  LogEvent,
  MetadataEvent,
  OutputInfo,
  StatusResponse,
  QueueItem,
  TrackListResponse,
  TrackSummary
} from "./types";
import AlbumDetailView from "./components/AlbumDetailView";
import AlbumsView from "./components/AlbumsView";
import FoldersView from "./components/FoldersView";
import OutputsModal from "./components/OutputsModal";
import PlayerBar from "./components/PlayerBar";
import QueueModal from "./components/QueueModal";
import SettingsView from "./components/SettingsView";
import SignalModal from "./components/SignalModal";
import {
  useLogsStream,
  useMetadataStream,
  useOutputsStream,
  useQueueStream,
  useStatusStream
} from "./hooks/streams";
import {usePlaybackActions} from "./hooks/usePlaybackActions";

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

interface LogEventEntry {
  id: number;
  event: LogEvent;
}

const MAX_METADATA_EVENTS = 200;
const MAX_LOG_EVENTS = 300;

function albumPlaceholder(title?: string | null, artist?: string | null): string {
  const source = title?.trim() || artist?.trim() || "";
  const initials = source
      .split(/\s+/)
      .map((part) => part.replace(/[^A-Za-z0-9]/g, ""))
      .filter(Boolean)
      .map((part) => part[0])
      .join("")
      .slice(0, 2)
      .toUpperCase();
  const label = initials || "NA";
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="#50555b"/><stop offset="100%" stop-color="#3f444a"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="18" y="32" font-family="Space Grotesk, sans-serif" font-size="28" fill="#ffffff" text-anchor="start">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
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

function normalizeMatch(value?: string | null): string {
  return value?.trim().toLowerCase() ?? "";
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
      return {title: "MusicBrainz batch", detail: `${event.count} candidates`};
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
      return {title: "MusicBrainz lookup failed", detail: event.error};
    case "cover_art_batch":
      return {title: "Cover art batch", detail: `${event.count} albums`};
    case "cover_art_fetch_start":
      return {title: "Cover art fetch started", detail: `album ${event.album_id}`};
    case "cover_art_fetch_success":
      return {title: "Cover art fetched", detail: `album ${event.album_id}`};
    case "cover_art_fetch_failure":
      return {
        title: "Cover art fetch failed",
        detail: `${event.error} (attempt ${event.attempts})`
      };
    default:
      return {title: "Metadata event"};
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
  const [settingsSection, setSettingsSection] = useState<"metadata" | "logs">("metadata");
  const [metadataEvents, setMetadataEvents] = useState<MetadataEventEntry[]>([]);
  const [logEvents, setLogEvents] = useState<LogEventEntry[]>([]);
  const [logsError, setLogsError] = useState<string | null>(null);
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
  const logIdRef = useRef(0);
  const metadataIdRef = useRef(0);

  const closeTrackMenu = useCallback(() => {
    setTrackMenuPath(null);
    setTrackMenuPosition(null);
  }, []);
  const toggleTrackMenu = useCallback(
      (path: string, target: Element) => {
        if (trackMenuPath === path) {
          closeTrackMenu();
          return;
        }
        const rect = target.getBoundingClientRect();
        setTrackMenuPosition({
          top: rect.bottom + 6,
          right: window.innerWidth - rect.right
        });
        setTrackMenuPath(path);
      },
      [trackMenuPath, closeTrackMenu]
  );
  const runTrackMenuAction = useCallback(
      (action: (path: string) => void | Promise<void>, path: string) => {
        action(path);
        closeTrackMenu();
      },
      [closeTrackMenu]
  );
  const handleClearLogs = useCallback(async () => {
    setLogEvents([]);
    try {
      await postJson<{ cleared_at_ms: number }>("/logs/clear");
      setLogsError(null);
    } catch (err) {
      setLogsError((err as Error).message);
    }
  }, []);
  const handleNavigateUp = useCallback(() => {
    if (!libraryDir) return;
    const parent = parentDir(libraryDir);
    if (parent) setLibraryDir(parent);
  }, [libraryDir]);
  const handleBackToRoot = useCallback(() => {
    setLibraryDir(null);
  }, []);

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
  const viewTitle = settingsOpen
      ? "Settings"
      : albumViewId !== null
          ? "Album"
          : browserView === "albums"
              ? "Albums"
              : "Folders";
  const playButtonTitle = !activeOutputId
      ? "Select an output to control playback."
      : !status?.now_playing && !selectedTrackPath
          ? "Select a track to play."
          : undefined;
  const selectedAlbum = useMemo(
      () => albums.find((album) => album.id === albumViewId) ?? null,
      [albums, albumViewId]
  );
  const activeAlbumId = useMemo(() => {
    const albumKey = normalizeMatch(status?.album);
    if (!albumKey) return null;
    const artistKey = normalizeMatch(status?.artist);
    const match = albums.find((album) => {
      if (normalizeMatch(album.title) !== albumKey) return false;
      if (!artistKey) return true;
      if (!album.artist) return true;
      return normalizeMatch(album.artist) === artistKey;
    });
    return match?.id ?? null;
  }, [albums, status?.album, status?.artist]);

  useEffect(() => {
    if (!trackMenuPath) return;

    function handleDocumentClick(event: MouseEvent) {
      const target = event.target as Element | null;
      if (target?.closest('[data-track-menu="true"]')) {
        return;
      }
      closeTrackMenu();
    }

    document.addEventListener("click", handleDocumentClick);
    return () => {
      document.removeEventListener("click", handleDocumentClick);
    };
  }, [trackMenuPath, closeTrackMenu]);

  useEffect(() => {
    if (!isPlaying && signalOpen) {
      setSignalOpen(false);
    }
  }, [isPlaying, signalOpen]);

  useOutputsStream({
    onEvent: (data) => {
      const activeId = data.outputs.some((output) => output.id === data.active_id)
          ? data.active_id
          : null;
      setOutputs(data.outputs);
      setActiveOutputId(activeId);
      setError(null);
    },
    onError: () => setError("Live outputs disconnected.")
  });

  const {
    handleRescanLibrary,
    handleRescanTrack,
    handlePause,
    handleNext,
    handleRescan,
    handleSelectOutput,
    handlePlay,
    handlePlayAlbumTrack,
    handlePlayAlbumById,
    handleQueueAlbumTrack,
    handlePlayAlbum,
    handleQueue,
    handlePlayNext
  } = usePlaybackActions({
    activeOutputId,
    albumTracks,
    rescanBusy,
    setError,
    setActiveOutputId,
    setRescanBusy
  });

  useMetadataStream({
    enabled: settingsOpen,
    onEvent: (event) => {
      const entry: MetadataEventEntry = {
        id: (metadataIdRef.current += 1),
        time: new Date(),
        event
      };
      setMetadataEvents((prev) => [entry, ...prev].slice(0, MAX_METADATA_EVENTS));
    },
    onError: () => setError("Live metadata updates disconnected.")
  });

  useLogsStream({
    enabled: settingsOpen,
    onSnapshot: (items) => {
      const entries = items
          .map((entry) => ({
            id: (logIdRef.current += 1),
            event: entry
          }))
          .reverse()
          .slice(0, MAX_LOG_EVENTS);
      setLogEvents(entries);
      setLogsError(null);
    },
    onEvent: (entry) => {
      const row: LogEventEntry = {
        id: (logIdRef.current += 1),
        event: entry
      };
      setLogEvents((prev) => [row, ...prev].slice(0, MAX_LOG_EVENTS));
    },
    onError: () => setLogsError("Live logs disconnected.")
  });

  useStatusStream({
    activeOutputId,
    onEvent: (data: SetStateAction<StatusResponse | null>) => {
      setStatus(data);
      setUpdatedAt(new Date());
      setError(null);
    },
    onError: () => setError("Live status disconnected.")
  });
  useEffect(() => {
    if (!activeOutputId) {
      setStatus(null);
    }
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

  useQueueStream({
    onEvent: (items) => {
      setQueue(items ?? []);
      setError(null);
    },
    onError: () => setError("Live queue disconnected.")
  });

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
        closeTrackMenu();
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
  }, [libraryDir, closeTrackMenu]);

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
            <div>
              <div className="nav-title">Audio Hub</div>
              <div className="nav-subtitle">Lossless control with a live signal view.</div>
            </div>
          </div>
          <div className="nav-section">
            <div className="nav-label">Library</div>
            <button
              className={`nav-button ${browserView === "albums" && !settingsOpen ? "active" : ""}`}
              onClick={() => {
                setAlbumViewId(null);
                setBrowserView("albums");
                setSettingsOpen(false);
              }}
            >
              <span className="nav-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <path
                    d="M4 4h16v12H4zM7 8h10v2H7zm0 4h6v2H7zM8 18h8v2H8z"
                    fill="currentColor"
                  />
                </svg>
              </span>
              <span>Albums</span>
            </button>
            <button
              className={`nav-button ${browserView === "library" && !settingsOpen ? "active" : ""}`}
              onClick={() => {
                setAlbumViewId(null);
                setBrowserView("library");
                setSettingsOpen(false);
              }}
            >
              <span className="nav-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <path
                    d="M3 7.5h7l2 2H21a1 1 0 0 1 1 1v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 4 17.5V8.5a1 1 0 0 1 1-1Z"
                    fill="currentColor"
                  />
                </svg>
              </span>
              <span>Folders</span>
            </button>
          </div>
          <div className="nav-section">
            <div className="nav-label">System</div>
            <button
              className={`nav-button ${settingsOpen ? "active" : ""}`}
              onClick={() => {
                setSettingsSection("metadata");
                setSettingsOpen(true);
              }}
            >
              <span className="nav-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24">
                  <path
                    d="M12 8.75a3.25 3.25 0 1 0 0 6.5 3.25 3.25 0 0 0 0-6.5Zm9.25 3.25c0-.5-.03-1-.1-1.48l2.02-1.57a.75.75 0 0 0 .17-.96l-1.92-3.32a.75.75 0 0 0-.91-.34l-2.38.96a9.5 9.5 0 0 0-2.56-1.48l-.36-2.52A.75.75 0 0 0 14.41 1h-3.82a.75.75 0 0 0-.74.64l-.36 2.52a9.5 9.5 0 0 0-2.56 1.48l-2.38-.96a.75.75 0 0 0-.91.34L1.72 8.34a.75.75 0 0 0 .17.96l2.02 1.57c-.07.48-.1.98-.1 1.48s.03 1 .1 1.48l-2.02 1.57a.75.75 0 0 0-.17.96l1.92 3.32a.75.75 0 0 0 .91.34l2.38-.96a9.5 9.5 0 0 0 2.56 1.48l.36 2.52a.75.75 0 0 0 .74.64h3.82a.75.75 0 0 0 .74-.64l.36-2.52a9.5 9.5 0 0 0 2.56-1.48l2.38.96a.75.75 0 0 0 .91-.34l1.92-3.32a.75.75 0 0 0-.17-.96l-2.02-1.57c.07-.48.1-.98.1-1.48Z"
                    fill="currentColor"
                  />
                </svg>
              </span>
              <span>Settings</span>
            </button>
          </div>
        </aside>

        <main className="main">
          <header className="view-header">
            <h1>{viewTitle}</h1>
            {error ? <div className="alert">{error}</div> : null}
          </header>

          {!settingsOpen && albumViewId === null ? (
            <section className="grid">
              {browserView === "albums" ? (
                <AlbumsView
                  albums={albums}
                  loading={albumsLoading}
                  error={albumsError}
                  placeholder={albumPlaceholder}
                  canPlay={Boolean(activeOutputId)}
                  activeAlbumId={activeAlbumId}
                  isPlaying={isPlaying}
                  onSelectAlbum={setAlbumViewId}
                  onPlayAlbum={handlePlayAlbumById}
                  onPause={handlePause}
                />
              ) : null}

              {browserView === "library" ? (
                <FoldersView
                  entries={libraryEntries}
                  dir={libraryDir}
                  loading={libraryLoading}
                  selectedTrackPath={selectedTrackPath}
                  trackMenuPath={trackMenuPath}
                  trackMenuPosition={trackMenuPosition}
                  canPlay={Boolean(activeOutputId)}
                  formatMs={formatMs}
                  onRescan={handleRescan}
                  onNavigateUp={handleNavigateUp}
                  onBackToRoot={handleBackToRoot}
                  onSelectDir={setLibraryDir}
                  onSelectTrack={setSelectedTrackPath}
                  onToggleMenu={toggleTrackMenu}
                  onPlay={(path) => runTrackMenuAction(handlePlay, path)}
                  onQueue={(path) => runTrackMenuAction(handleQueue, path)}
                  onPlayNext={(path) => runTrackMenuAction(handlePlayNext, path)}
                  onRescanTrack={(path) => runTrackMenuAction(handleRescanTrack, path)}
                />
              ) : null}
            </section>
          ) : null}

          {albumViewId !== null && !settingsOpen ? (
            <AlbumDetailView
              album={selectedAlbum}
              tracks={albumTracks}
              loading={albumTracksLoading}
              error={albumTracksError}
              placeholder={albumPlaceholder}
              canPlay={Boolean(activeOutputId) && albumTracks.length > 0}
              formatMs={formatMs}
              onBack={() => setAlbumViewId(null)}
              onPlayAlbum={handlePlayAlbum}
              onPlayTrack={handlePlayAlbumTrack}
              onQueueTrack={handleQueueAlbumTrack}
            />
          ) : null}

          <SettingsView
            active={settingsOpen}
            section={settingsSection}
            onSectionChange={setSettingsSection}
            metadataEvents={metadataEvents}
            logEvents={logEvents}
            logsError={logsError}
            rescanBusy={rescanBusy}
            onClearMetadata={() => setMetadataEvents([])}
            onRescanLibrary={handleRescanLibrary}
            onClearLogs={handleClearLogs}
            describeMetadataEvent={describeMetadataEvent}
            metadataDetailLines={metadataDetailLines}
          />
        </main>
      </div>

      <div className={settingsOpen ? "hidden" : ""}>
        <PlayerBar
          status={status}
          nowPlayingCover={nowPlayingCover}
          nowPlayingCoverFailed={nowPlayingCoverFailed}
          isPlaying={isPlaying}
          canTogglePlayback={canTogglePlayback}
          showPlayIcon={showPlayIcon}
          playButtonTitle={playButtonTitle}
          queueHasItems={queue.length > 0}
          activeOutput={activeOutput}
          uiBuildId={uiBuildId}
          formatMs={formatMs}
          onCoverError={() => setNowPlayingCoverFailed(true)}
          onPrimaryAction={handlePrimaryAction}
          onNext={handleNext}
          onSignalOpen={() => setSignalOpen(true)}
          onQueueOpen={() => setQueueOpen(true)}
          onSelectOutput={() => setOutputsOpen(true)}
        />
      </div>

      <OutputsModal
        open={outputsOpen}
        outputs={outputs}
        activeOutputId={activeOutputId}
        onClose={() => setOutputsOpen(false)}
        onSelectOutput={handleSelectOutput}
        formatRateRange={formatRateRange}
      />

      <SignalModal
        open={signalOpen}
        status={status}
        activeOutput={activeOutput}
        updatedAt={updatedAt}
        formatHz={formatHz}
        onClose={() => setSignalOpen(false)}
      />

      <QueueModal
        open={queueOpen}
        items={queue}
        onClose={() => setQueueOpen(false)}
        formatMs={formatMs}
      />

    </div>
  );
}
