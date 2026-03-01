import { useEffect, useMemo, useState, useCallback, useRef, SetStateAction} from "react";
import {
  fetchJson,
  postJson,
} from "./api";
import {
  LogEvent,
  MetadataEvent,
  OutputInfo,
  SessionCreateResponse,
  SessionVolumeResponse,
  StatusResponse,
  QueueItem,
  TrackListResponse,
  TrackSummary
} from "./types";
import AppModals from "./components/AppModals";
import AppChrome from "./components/AppChrome";
import MainContent from "./components/MainContent";
import {
  useLogsStream,
  useMetadataStream,
  useOutputsStream,
  useQueueStream,
  useStatusStream
} from "./hooks/streams";
import { usePlaybackActions } from "./hooks/usePlaybackActions";
import { useQueueActions } from "./hooks/useQueueActions";
import { useHubConnection } from "./hooks/useHubConnection";
import { useAlbumsState } from "./hooks/useAlbumsState";
import { useLocalPlayback } from "./hooks/useLocalPlayback";
import { useNowPlayingCover } from "./hooks/useNowPlayingCover";
import { useOutputSettings } from "./hooks/useOutputSettings";
import { useSessionsState } from "./hooks/useSessionsState";
import { useTrackMenu } from "./hooks/useTrackMenu";
import { useToasts } from "./hooks/useToasts";
import { SettingsSection, useViewNavigation } from "./hooks/useViewNavigation";

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

interface LogEventEntry {
  id: number;
  event: LogEvent;
}

type MatchTarget = {
  trackId?: number;
  title: string;
  artist: string;
  album?: string | null;
};

type EditTarget = {
  trackId?: number;
  label: string;
  defaults: {
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    year?: number | null;
    trackNumber?: number | null;
    discNumber?: number | null;
  };
};

type AlbumEditTarget = {
  albumId: number;
  label: string;
  artist: string;
  defaults: {
    title?: string | null;
    albumArtist?: string | null;
    year?: number | null;
  };
};

const MAX_METADATA_EVENTS = 200;
const MAX_LOG_EVENTS = 300;
const WEB_SESSION_CLIENT_ID_KEY = "audioHub.webSessionClientId";
const WEB_SESSION_ID_KEY = "audioHub.webSessionId";
const NAV_COLLAPSED_KEY = "audioHub.navCollapsed";
const WEB_DEFAULT_SESSION_NAME = "Default";

function isDefaultSessionName(name: string | null | undefined): boolean {
  return (name ?? "").trim().toLowerCase() === WEB_DEFAULT_SESSION_NAME.toLowerCase();
}

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
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="#1a1d23"/><stop offset="100%" stop-color="#0f1215"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="18" y="32" font-family="Space Grotesk, sans-serif" font-size="28" fill="#d4965f" text-anchor="start">${label}</text></svg>`;
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

// Folders view removed.

function describeMetadataEvent(event: MetadataEvent): { title: string; detail?: string } {
  switch (event.kind) {
    case "library_scan_album_start":
      return {title: "Scanning album folder", detail: event.album};
    case "library_scan_album_finish":
      return {title: "Scanned album folder", detail: `${event.tracks} tracks`};
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
    if (event.kind === "library_scan_album_finish") {
      return [event.album];
    }
    if (event.kind === "library_scan_album_start") {
      return [event.album];
    }
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

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

function getOrCreateWebSessionClientId(): string {
  try {
    const existing = localStorage.getItem(WEB_SESSION_CLIENT_ID_KEY);
    if (existing) return existing;
    const generated =
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    localStorage.setItem(WEB_SESSION_CLIENT_ID_KEY, generated);
    return generated;
  } catch {
    return `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  }
}

export default function App() {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [createSessionOpen, setCreateSessionOpen] = useState<boolean>(false);
  const [newSessionName, setNewSessionName] = useState<string>("");
  const [newSessionNeverExpires, setNewSessionNeverExpires] = useState<boolean>(false);
  const [createSessionBusy, setCreateSessionBusy] = useState<boolean>(false);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [sessionVolume, setSessionVolume] = useState<SessionVolumeResponse | null>(null);
  const [volumeBusy, setVolumeBusy] = useState<boolean>(false);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [rescanBusy, setRescanBusy] = useState<boolean>(false);
  const [queueOpen, setQueueOpen] = useState<boolean>(false);
  const [signalOpen, setSignalOpen] = useState<boolean>(false);
  const [outputsOpen, setOutputsOpen] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [catalogOpen, setCatalogOpen] = useState<boolean>(false);
  const [albumNotesOpen, setAlbumNotesOpen] = useState<boolean>(false);
  const [analysisTarget, setAnalysisTarget] = useState<{
    trackId: number;
    title: string;
    artist?: string | null;
  } | null>(null);
  const [navCollapsed, setNavCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(NAV_COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("metadata");
  const [metadataEvents, setMetadataEvents] = useState<MetadataEventEntry[]>([]);
  const [logEvents, setLogEvents] = useState<LogEventEntry[]>([]);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [albumSearch, setAlbumSearch] = useState<string>("");
  const [albumViewMode, setAlbumViewMode] = useState<"grid" | "list">("grid");
  const [albumViewId, setAlbumViewId] = useState<number | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const [matchTarget, setMatchTarget] = useState<MatchTarget | null>(null);
  const [editTarget, setEditTarget] = useState<EditTarget | null>(null);
  const [albumEditTarget, setAlbumEditTarget] = useState<AlbumEditTarget | null>(null);
  const logIdRef = useRef(0);
  const metadataIdRef = useRef(0);
  const activeSessionIdRef = useRef<string | null>(null);
  const isLocalSessionRef = useRef<boolean>(false);
  const volumeRequestSeqRef = useRef(0);
  const handleClearLogs = useCallback(async () => {
    setLogEvents([]);
    try {
      await postJson<{ cleared_at_ms: number }>("/logs/clear");
      setLogsError(null);
    } catch (err) {
      setLogsError((err as Error).message);
    }
  }, []);
  const {
    notifications,
    notificationsOpen,
    unreadCount,
    reportError,
    clearNotifications,
    toggleNotifications
  } = useToasts();
  const {
    serverConnected,
    serverConnecting,
    serverError,
    apiBaseOverride,
    apiBaseDefault,
    handleApiBaseChange,
    handleApiBaseReset,
    connectionError,
    markServerConnected
  } = useHubConnection();
  const {
    sessionId,
    setSessionId,
    sessions,
    sessionOutputLocks,
    sessionBridgeLocks,
    activeOutputId,
    setActiveOutputId,
    refreshSessions,
    refreshSessionLocks,
    refreshSessionDetail,
    selectSession
  } = useSessionsState({
    serverConnected,
    apiBaseOverride,
    appVersion: __APP_VERSION__,
    getClientId: getOrCreateWebSessionClientId,
    sessionStorageKey: WEB_SESSION_ID_KEY,
    onError: reportError
  });
  const streamKey = useMemo(
    () => `${apiBaseOverride}:${serverConnected ? "up" : "down"}:${sessionId ?? "none"}`,
    [apiBaseOverride, serverConnected, sessionId]
  );
  const {
    albums,
    albumsLoading,
    albumsError,
    setAlbumsError,
    albumTracks,
    albumTracksLoading,
    albumTracksError,
    setAlbumTracksError,
    albumProfile,
    setAlbumProfile,
    catalogLoading,
    catalogError,
    loadAlbums,
    loadAlbumTracks,
    loadCatalogProfiles
  } = useAlbumsState({
    serverConnected,
    streamKey,
    albumViewId,
    connectionError,
    markServerConnected
  });
  const { navigateTo, canGoBack, canGoForward, goBack, goForward } = useViewNavigation({
    setSettingsOpen,
    setAlbumViewId,
    setSettingsSection
  });
  const { trackMenuTrackId, trackMenuPosition, toggleTrackMenu, runTrackMenuAction } =
    useTrackMenu();
  const {
    outputsSettings,
    outputsProviders,
    outputsLoading,
    outputsError,
    outputsLastRefresh,
    handleRefreshProvider,
    handleToggleOutputSetting,
    handleRenameOutputSetting,
    handleToggleExclusiveSetting
  } = useOutputSettings({
    settingsOpen,
    settingsSection,
    serverConnected
  });
  const activeOutput = useMemo(
      () => outputs.find((output) => output.id === activeOutputId) ?? null,
      [outputs, activeOutputId]
  );
  const currentSession = useMemo(
    () => sessions.find((session) => session.id === sessionId) ?? null,
    [sessions, sessionId]
  );
  const isLocalSession = currentSession?.mode === "local";
  const {
    audioRef,
    applyLocalPlayback,
    requestLocalCommand,
    toggleLocalPause,
    resumeLocalFromStatus
  } = useLocalPlayback({
    isLocalSession: Boolean(isLocalSession),
    sessionId,
    activeOutputId,
    queue,
    status,
    setStatus,
    markUpdatedAt: () => setUpdatedAt(new Date()),
    reportError: (message) => reportError(message)
  });
  const {
    nowPlayingCover,
    nowPlayingCoverFailed,
    nowPlayingAlbumId,
    onCoverError
  } = useNowPlayingCover(status?.now_playing_track_id ?? null);
  const queueNowPlayingTrackId = useMemo(() => {
    const item = queue.find((entry) => entry.kind === "track" && Boolean(entry.now_playing));
    return item?.kind === "track" ? item.id : null;
  }, [queue]);
  const replayTrackId = useMemo(() => {
    const playedTracks = queue.filter(
      (entry): entry is QueueItem & { kind: "track" } =>
        entry.kind === "track" && Boolean(entry.played) && Number.isFinite(entry.id)
    );
    if (!playedTracks.length) return null;
    return playedTracks[playedTracks.length - 1].id;
  }, [queue]);
  const hasPlayedHistory = Boolean(replayTrackId);
  const staleEndedStatus =
    !isLocalSession && hasPlayedHistory && queueNowPlayingTrackId === null;
  const effectiveNowPlayingTrackId = staleEndedStatus
    ? null
    : queueNowPlayingTrackId ?? status?.now_playing_track_id ?? null;
  const hasNowPlaying = effectiveNowPlayingTrackId !== null;
  const canReplayFromHistory = Boolean(
    sessionId &&
      (isLocalSession || activeOutputId) &&
      !hasNowPlaying &&
      replayTrackId
  );
  const canTogglePlayback = Boolean(
    sessionId &&
      (isLocalSession || activeOutputId) &&
      (hasNowPlaying || canReplayFromHistory)
  );
  const canControlVolume = Boolean(serverConnected && sessionId && !isLocalSession && activeOutputId);
  const isPlaying = Boolean(hasNowPlaying && !status?.paused);
  const isPaused = Boolean(!hasNowPlaying || status?.paused);
  const uiBuildId = useMemo(() => {
    if (__BUILD_MODE__ === "development") {
      return "dev";
    }
    return `v${__APP_VERSION__}+${__GIT_SHA__}`;
  }, []);

  const viewTitle = settingsOpen ? "Settings" : albumViewId !== null ? "" : "Albums";
  const playButtonTitle = !sessionId
    ? "Creating session..."
    : !activeOutputId && !isLocalSession
    ? (isLocalSession
      ? "Local session is ready."
      : "Select an output to control playback.")
    : !hasNowPlaying
      ? canReplayFromHistory
        ? "Replay the last track."
        : "Select an album track to play."
      : undefined;
  const selectedAlbum = useMemo(
      () => albums.find((album) => album.id === albumViewId) ?? null,
      [albums, albumViewId]
  );
  const filteredAlbums = useMemo(() => {
    const query = albumSearch.trim().toLowerCase();
    if (!query) return albums;
    return albums.filter((album) => {
      const title = album.title?.toLowerCase() ?? "";
      const artist = album.artist?.toLowerCase() ?? "";
      const year = album.year ? String(album.year) : "";
      const originalYear = album.original_year ? String(album.original_year) : "";
      const editionYear = album.edition_year ? String(album.edition_year) : "";
      const editionLabel = album.edition_label?.toLowerCase() ?? "";
      return (
        title.includes(query) ||
        artist.includes(query) ||
        year.includes(query) ||
        originalYear.includes(query) ||
        editionYear.includes(query) ||
        editionLabel.includes(query)
      );
    });
  }, [albums, albumSearch]);
  const heuristicAlbumId = useMemo(() => {
    const albumKey = normalizeMatch(status?.album);
    if (!albumKey) return null;
    const artistKey = normalizeMatch(status?.artist);
    const allowArtistMismatch = (albumArtist?: string | null) => {
      if (!albumArtist) return true;
      const key = normalizeMatch(albumArtist);
      return key === "various artists" || key === "various" || key === "va";
    };
    const match = albums.find((album) => {
      if (normalizeMatch(album.title) !== albumKey) return false;
      if (!artistKey) return true;
      if (!album.artist) return true;
      if (normalizeMatch(album.artist) === artistKey) return true;
      return allowArtistMismatch(album.artist);
    });
    return match?.id ?? null;
  }, [albums, status?.album, status?.artist]);
  const activeAlbumId = nowPlayingAlbumId ?? heuristicAlbumId;

  useEffect(() => {
    try {
      localStorage.setItem(NAV_COLLAPSED_KEY, navCollapsed ? "1" : "0");
    } catch {
      // ignore storage failures
    }
  }, [navCollapsed]);

  useEffect(() => {
    if (!serverConnected) return;
    setAlbumsError(null);
    setAlbumTracksError(null);
  }, [serverConnected, setAlbumTracksError, setAlbumsError]);

  useEffect(() => {
    activeSessionIdRef.current = sessionId;
    isLocalSessionRef.current = Boolean(isLocalSession);
  }, [isLocalSession, sessionId]);

  const openTrackMatchForAlbum = useCallback(
    (trackId: number) => {
      const track = albumTracks.find((item) => item.id === trackId);
      const title = track?.title ?? track?.file_name ?? "Unknown track";
      const artist = track?.artist ?? "Unknown artist";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      setMatchTarget({
        trackId: track?.id ?? trackId,
        title,
        artist,
        album
      });
    },
    [albumTracks, selectedAlbum]
  );

  const openAlbumEditor = useCallback(() => {
    if (!selectedAlbum) return;
    const label = selectedAlbum.artist
      ? `${selectedAlbum.title} — ${selectedAlbum.artist}`
      : selectedAlbum.title;
    setAlbumEditTarget({
      albumId: selectedAlbum.id,
      label,
      artist: selectedAlbum.artist ?? "Unknown artist",
      defaults: {
        title: selectedAlbum.title,
        albumArtist: selectedAlbum.artist ?? null,
        year: selectedAlbum.year ?? null
      }
    });
  }, [selectedAlbum]);

  const openTrackEditorForAlbum = useCallback(
    (trackId: number) => {
      const track = albumTracks.find((item) => item.id === trackId);
      const title = track?.title ?? track?.file_name ?? "Unknown track";
      const artist = track?.artist ?? "";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      const label = artist ? `${title} — ${artist}` : title;
      setEditTarget({
        trackId: track?.id ?? trackId,
        label,
        defaults: {
          title,
          artist,
          album,
          albumArtist: selectedAlbum?.artist ?? null,
          trackNumber: track?.track_number ?? null,
          discNumber: track?.disc_number ?? null
        }
      });
    },
    [albumTracks, selectedAlbum]
  );

  const matchLabel = matchTarget
    ? `${matchTarget.title}${matchTarget.artist ? ` — ${matchTarget.artist}` : ""}`
    : "";
  const matchDefaults = matchTarget
    ? {
        title: matchTarget.title,
        artist: matchTarget.artist,
        album: matchTarget.album ?? ""
      }
    : { title: "", artist: "", album: "" };
  const editLabel = editTarget?.label ?? "";
  const editDefaults = editTarget?.defaults ?? {};
  const albumEditLabel = albumEditTarget?.label ?? "";
  const albumEditDefaults = albumEditTarget?.defaults ?? {};

  useEffect(() => {
    if (!status?.now_playing_track_id && signalOpen) {
      setSignalOpen(false);
    }
  }, [status?.now_playing_track_id, signalOpen]);

  const refreshSessionVolume = useCallback(
    async (id: string, silent = true) => {
      try {
        const volume = await fetchJson<SessionVolumeResponse>(
          `/sessions/${encodeURIComponent(id)}/volume`
        );
        setSessionVolume(volume);
      } catch (err) {
        setSessionVolume(null);
        if (!silent) {
          reportError((err as Error).message);
        }
      }
    },
    [reportError]
  );

  const handleSessionChange = useCallback(
    async (nextSessionId: string) => {
      setStatus(null);
      setSessionVolume(null);
      setQueue([]);
      try {
        await selectSession(nextSessionId);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [reportError, selectSession]
  );

  const createNamedSession = useCallback(async (name: string, neverExpires = false) => {
    try {
      const response = await postJson<SessionCreateResponse>("/sessions", {
        name,
        mode: "remote",
        client_id:
          typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
            ? `${getOrCreateWebSessionClientId()}:${crypto.randomUUID()}`
            : `${getOrCreateWebSessionClientId()}-${Date.now()}`,
        app_version: __APP_VERSION__,
        owner: "web-ui",
        ...(neverExpires ? { lease_ttl_sec: 0 } : {})
      });
      await Promise.all([refreshSessions(), refreshSessionLocks()]);
      await handleSessionChange(response.session_id);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [refreshSessionLocks, refreshSessions, handleSessionChange, reportError]);

  const handleCreateSession = useCallback(() => {
    setNewSessionName(`Session ${sessions.length + 1}`);
    setNewSessionNeverExpires(false);
    setCreateSessionOpen(true);
  }, [sessions.length]);

  const submitCreateSession = useCallback(async () => {
    const name = newSessionName.trim();
    if (!name) {
      reportError("Session name is required.");
      return;
    }
    setCreateSessionBusy(true);
    try {
      await createNamedSession(name, newSessionNeverExpires);
      setCreateSessionOpen(false);
      setNewSessionName("");
      setNewSessionNeverExpires(false);
    } finally {
      setCreateSessionBusy(false);
    }
  }, [createNamedSession, newSessionName, newSessionNeverExpires, reportError]);

  const handleDeleteSession = useCallback(async () => {
    if (!sessionId) return;
    const session = sessions.find((item) => item.id === sessionId) ?? null;
    if (!session || isDefaultSessionName(session.name)) {
      return;
    }
    const confirmed = window.confirm(`Delete session "${session.name}"?`);
    if (!confirmed) return;

    try {
      await fetchJson(`/sessions/${encodeURIComponent(sessionId)}`, {
        method: "DELETE"
      });
      const nextSessions = await refreshSessions();
      await refreshSessionLocks();
      const defaultSession =
        nextSessions.find((item) => isDefaultSessionName(item.name)) ?? nextSessions[0] ?? null;
      if (defaultSession) {
        await handleSessionChange(defaultSession.id);
      } else {
        setSessionId(null);
        setActiveOutputId(null);
        setStatus(null);
        setSessionVolume(null);
        setQueue([]);
        try {
          localStorage.removeItem(WEB_SESSION_ID_KEY);
        } catch {
          // ignore storage failures
        }
      }
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [sessionId, sessions, refreshSessionLocks, refreshSessions, handleSessionChange, reportError]);

  useEffect(() => {
    if (!canControlVolume || !sessionId) {
      setSessionVolume(null);
      return;
    }
    volumeRequestSeqRef.current += 1;
    refreshSessionVolume(sessionId, true);
  }, [activeOutputId, canControlVolume, refreshSessionVolume, sessionId]);

  useOutputsStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    onEvent: (data) => {
      setOutputs(data.outputs);
      const sid = activeSessionIdRef.current;
      if (sid) {
        refreshSessionDetail(sid).catch(() => {
          // Best-effort session output sync for cross-client output switches.
        });
      }
      markServerConnected();
    },
    onError: () => {
      const message = connectionError("Live outputs disconnected", "/outputs/stream");
      reportError(message, "warn");
    }
  });

  const {
    handleRescanLibrary,
    handleRescanTrack,
    handlePause: handlePauseRemote,
    handleSelectOutput,
    handlePlay: handlePlayRemote,
    handlePlayAlbumTrack: handlePlayAlbumTrackRemote,
    handlePlayAlbumById: handlePlayAlbumByIdRemote
  } = usePlaybackActions({
    sessionId,
    activeOutputId,
    rescanBusy,
    setError: reportError,
    setActiveOutputId,
    setRescanBusy
  });
  const {
    handleNext: handleNextRemote,
    handlePrevious: handlePreviousRemote,
    handleQueue,
    handlePlayNext,
    handleQueueClear,
    handleQueuePlayFrom: handleQueuePlayFromRemote
  } = useQueueActions({ sessionId, setError: reportError });

  const handlePause = useCallback(async () => {
    if (isLocalSession) {
      await toggleLocalPause();
      return;
    }
    await handlePauseRemote();
  }, [handlePauseRemote, isLocalSession, toggleLocalPause]);

  const handleSetVolume = useCallback(
    async (value: number) => {
      if (!sessionId || isLocalSession || !activeOutputId) return;
      const clamped = Math.max(0, Math.min(100, Math.round(value)));
      setSessionVolume((prev) => ({
        value: clamped,
        muted: prev?.muted ?? false,
        source: prev?.source ?? "bridge",
        available: true
      }));
      const requestSeq = ++volumeRequestSeqRef.current;
      try {
        const payload = await postJson<SessionVolumeResponse>(
          `/sessions/${encodeURIComponent(sessionId)}/volume`,
          { value: clamped }
        );
        if (requestSeq === volumeRequestSeqRef.current) {
          setSessionVolume(payload);
        }
      } catch (err) {
        if (requestSeq !== volumeRequestSeqRef.current) {
          return;
        }
        reportError((err as Error).message);
        await refreshSessionVolume(sessionId, true);
      }
    },
    [activeOutputId, isLocalSession, refreshSessionVolume, reportError, sessionId]
  );

  const handleToggleMute = useCallback(async () => {
    if (!sessionId || isLocalSession || !activeOutputId || !sessionVolume) return;
    const nextMuted = !Boolean(sessionVolume.muted);
    setVolumeBusy(true);
    setSessionVolume({ ...sessionVolume, muted: nextMuted });
    try {
      const payload = await postJson<SessionVolumeResponse>(
        `/sessions/${encodeURIComponent(sessionId)}/mute`,
        { muted: nextMuted }
      );
      setSessionVolume(payload);
    } catch (err) {
      reportError((err as Error).message);
      await refreshSessionVolume(sessionId, true);
    } finally {
      setVolumeBusy(false);
    }
  }, [
    activeOutputId,
    isLocalSession,
    refreshSessionVolume,
    reportError,
    sessionId,
    sessionVolume
  ]);

  const handlePlay = useCallback(
    async (trackId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayRemote(trackId);
          return;
        }
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, { track_ids: [trackId] });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [applyLocalPlayback, handlePlayRemote, isLocalSession, reportError, requestLocalCommand, sessionId]
  );

  const handleNext = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handleNextRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/next");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [applyLocalPlayback, handleNextRemote, isLocalSession, reportError, requestLocalCommand]);

  const handlePrevious = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handlePreviousRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/previous");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [applyLocalPlayback, handlePreviousRemote, isLocalSession, reportError, requestLocalCommand]);

  const handleQueuePlayFrom = useCallback(
    async (trackId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handleQueuePlayFromRemote(trackId);
          return;
        }
        const command = await requestLocalCommand("/queue/play_from", { track_id: trackId });
        await applyLocalPlayback(command);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [applyLocalPlayback, handleQueuePlayFromRemote, isLocalSession, reportError, requestLocalCommand, sessionId]
  );

  const handlePlayAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumTrackRemote(track);
          return;
        }
        if (!track.id) {
          return;
        }
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, {
          track_ids: [track.id]
        });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      applyLocalPlayback,
      handlePlayAlbumTrackRemote,
      isLocalSession,
      reportError,
      requestLocalCommand,
      sessionId
    ]
  );

  const handlePlayAlbumById = useCallback(
    async (albumId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumByIdRemote(albumId);
          return;
        }
        const tracks = await fetchJson<TrackListResponse>(`/tracks?album_id=${albumId}&limit=500`);
        const trackIds = (tracks.items ?? [])
          .map((track) => track.id)
          .filter((id): id is number => Number.isFinite(id));
        if (!trackIds.length) {
          throw new Error("Album has no playable tracks.");
        }
        const base = `/sessions/${encodeURIComponent(sessionId)}/queue`;
        await postJson(`${base}/clear`, {
          clear_queue: true,
          clear_history: false
        });
        await postJson(base, { track_ids: trackIds });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      applyLocalPlayback,
      handlePlayAlbumByIdRemote,
      isLocalSession,
      reportError,
      requestLocalCommand,
      sessionId
    ]
  );

  const handleSelectOutputForSession = useCallback(
    async (id: string) => {
      if (isLocalSession) return;
      await handleSelectOutput(id, false);
      if (!sessionId) return;
      try {
        await Promise.all([refreshSessions(), refreshSessionLocks(), refreshSessionDetail(sessionId)]);
      } catch {
        // best-effort refresh
      }
    },
    [
      handleSelectOutput,
      isLocalSession,
      refreshSessionDetail,
      refreshSessionLocks,
      refreshSessions,
      sessionId
    ]
  );

  useMetadataStream({
    enabled: settingsOpen && serverConnected && settingsSection === "metadata",
    onEvent: (event) => {
      const entry: MetadataEventEntry = {
        id: (metadataIdRef.current += 1),
        time: new Date(),
        event
      };
      setMetadataEvents((prev) => [entry, ...prev].slice(0, MAX_METADATA_EVENTS));
    },
    onError: () =>
      reportError(connectionError("Live metadata updates disconnected", "/metadata/stream"), "warn")
  });

  useLogsStream({
    enabled: settingsOpen && serverConnected && settingsSection === "logs",
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
    onError: () => {
      const message = connectionError("Live logs disconnected", "/logs/stream");
      setLogsError(message);
      reportError(message, "warn");
    }
  });

  useStatusStream({
    enabled: serverConnected && !isLocalSession && Boolean(sessionId && activeOutputId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (data: SetStateAction<StatusResponse | null>) => {
      if (isLocalSessionRef.current) {
        return;
      }
      if (!sessionId || activeSessionIdRef.current !== sessionId) {
        return;
      }
      setStatus(data);
      setUpdatedAt(new Date());
      markServerConnected();
    },
    onError: () => {
      if (!activeOutputId) {
        return;
      }
      const message = connectionError(
        "Live status disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/status/stream`
          : "/sessions/{id}/status/stream"
      );
      reportError(message, "warn");
    }
  });

  useQueueStream({
    enabled: serverConnected && Boolean(sessionId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (items) => {
      setQueue(items ?? []);
      markServerConnected();
    },
    onError: () => {
      const message = connectionError(
        "Live queue disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/queue/stream`
          : "/sessions/{id}/queue/stream"
      );
      reportError(message, "warn");
    }
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
    setAlbumNotesOpen(false);
  }, [albumViewId]);

  useEffect(() => {
    if (albumViewId === null) return;
    const main = document.querySelector<HTMLElement>(".main");
    if (main) {
      main.scrollTo({ top: 0, behavior: "smooth" });
    } else {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  }, [albumViewId]);


  const handlePlayMedia = useCallback(async () => {
    if (hasNowPlaying) {
      if (status.paused) {
        await handlePause();
      }
      return;
    }
    if (replayTrackId !== null) {
      await handleQueuePlayFrom(replayTrackId);
    }
  }, [handlePause, handleQueuePlayFrom, hasNowPlaying, replayTrackId, status?.paused]);

  const handlePauseMedia = useCallback(async () => {
    if (hasNowPlaying && !status?.paused) {
      await handlePause();
    }
  }, [handlePause, hasNowPlaying, status?.paused]);

  useEffect(() => {
    function handleKey(event: KeyboardEvent) {
      if (event.code !== "Space") return;
      if (event.repeat) return;
      if (isEditableTarget(event.target)) return;
      event.preventDefault();
      handlePrimaryAction();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [handlePrimaryAction]);

  useEffect(() => {
    const session = navigator.mediaSession;
    if (!session) return;

    if (status?.title || status?.artist || status?.album) {
      const artwork = nowPlayingCover ? [{ src: nowPlayingCover, sizes: "512x512" }] : [];
      session.metadata = new MediaMetadata({
        title: status?.title ?? "",
        artist: status?.artist ?? "",
        album: status?.album ?? "",
        artwork
      });
    } else {
      session.metadata = null;
    }

    try {
      session.setActionHandler("play", () => {
        handlePlayMedia();
      });
      session.setActionHandler("pause", () => {
        handlePauseMedia();
      });
      session.setActionHandler("previoustrack", () => {
        handlePrevious();
      });
      session.setActionHandler("nexttrack", () => {
        handleNext();
      });
    } catch {
      // MediaSession action handlers are best-effort.
    }

    return () => {
      try {
        session.setActionHandler("play", null);
        session.setActionHandler("pause", null);
        session.setActionHandler("previoustrack", null);
        session.setActionHandler("nexttrack", null);
      } catch {
        // Best-effort cleanup.
      }
    };
  }, [
    handleNext,
    handlePauseMedia,
    handlePlayMedia,
    handlePrevious,
    nowPlayingCover,
    status?.album,
    status?.artist,
    status?.title
  ]);

  async function handlePrimaryAction() {
    if (hasNowPlaying) {
      if (isLocalSession && status.paused && sessionId) {
        try {
          const currentQueueTrack = queue.find(
            (item) => item.kind === "track" && item.now_playing && item.id
          ) as { id?: number } | undefined;
          const replayId = currentQueueTrack?.id ?? effectiveNowPlayingTrackId;
          const resumed = await resumeLocalFromStatus(replayId ?? null, status.elapsed_ms ?? null);
          if (resumed) {
            return;
          }
        } catch (err) {
          reportError((err as Error).message);
          return;
        }
      }
      await handlePause();
      return;
    }
    if (replayTrackId !== null) {
      await handleQueuePlayFrom(replayTrackId);
    }
  }

  const showGate = !serverConnected;
  const queueHasNext = Boolean(sessionId && (activeOutputId || isLocalSession)) && queue.some((item) =>
    item.kind === "track" ? !item.now_playing : true
  );
  const deleteSessionDisabled =
    !serverConnected ||
    !sessionId ||
    sessions.find((item) => item.id === sessionId)?.mode === "local" ||
    isDefaultSessionName(sessions.find((item) => item.id === sessionId)?.name);
  const canGoPrevious = isLocalSession
    ? queue.some((item) => item.kind === "track" && Boolean(item.played))
    : Boolean(status?.has_previous);
  return (
    <AppChrome
      settingsOpen={settingsOpen}
      showGate={showGate}
      navCollapsed={navCollapsed}
      onToggleNavCollapsed={() => setNavCollapsed((prev) => !prev)}
      navigateTo={navigateTo}
      serverConnecting={serverConnecting}
      serverError={serverError}
      apiBaseOverride={apiBaseOverride}
      apiBaseDefault={apiBaseDefault}
      onApiBaseChange={handleApiBaseChange}
      onApiBaseReset={handleApiBaseReset}
      canGoBack={canGoBack}
      canGoForward={canGoForward}
      onGoBack={goBack}
      onGoForward={goForward}
      viewTitle={viewTitle}
      albumViewId={albumViewId}
      albumSearch={albumSearch}
      onAlbumSearchChange={setAlbumSearch}
      albumViewMode={albumViewMode}
      onAlbumViewModeChange={setAlbumViewMode}
      sessionId={sessionId}
      sessions={sessions}
      serverConnected={serverConnected}
      onSessionChange={handleSessionChange}
      onCreateSession={handleCreateSession}
      onDeleteSession={() => {
        void handleDeleteSession();
      }}
      deleteSessionDisabled={deleteSessionDisabled}
      notificationsOpen={notificationsOpen}
      unreadCount={unreadCount}
      notifications={notifications}
      onToggleNotifications={toggleNotifications}
      onClearNotifications={clearNotifications}
      playerVisible={!showGate}
      playerStatus={status}
      playerUpdatedAt={updatedAt}
      nowPlayingCover={nowPlayingCover}
      nowPlayingCoverFailed={nowPlayingCoverFailed}
      isLocalSession={Boolean(isLocalSession)}
      hasNowPlaying={hasNowPlaying}
      canTogglePlayback={canTogglePlayback}
      canGoPrevious={canGoPrevious}
      isPaused={isPaused}
      playButtonTitle={playButtonTitle}
      queueHasNext={queueHasNext}
      queueOpen={queueOpen}
      sessionVolume={sessionVolume}
      volumeBusy={volumeBusy}
      activeOutput={activeOutput}
      activeAlbumId={activeAlbumId}
      uiBuildId={uiBuildId}
      formatMs={formatMs}
      albumPlaceholder={albumPlaceholder}
      onCoverError={onCoverError}
      onAlbumNavigate={(albumId) =>
        navigateTo({
          view: "album",
          albumId
        })
      }
      onPrimaryAction={handlePrimaryAction}
      onPrevious={handlePrevious}
      onNext={handleNext}
      onSignalOpen={() => setSignalOpen(true)}
      onQueueOpen={() => setQueueOpen((value) => !value)}
      onVolumeChange={handleSetVolume}
      onVolumeToggleMute={handleToggleMute}
      onSelectOutput={() => {
        if (!isLocalSession) {
          setOutputsOpen(true);
        }
      }}
      mainContent={
        <MainContent
          settingsOpen={settingsOpen}
          albumViewId={albumViewId}
          filteredAlbums={filteredAlbums}
          albumsLoading={albumsLoading}
          albumsError={albumsError}
          placeholder={albumPlaceholder}
          sessionId={sessionId}
          activeOutputId={activeOutputId}
          isLocalSession={Boolean(isLocalSession)}
          activeAlbumId={activeAlbumId}
          isPlaying={isPlaying}
          isPaused={isPaused}
          albumViewMode={albumViewMode}
          onSelectAlbum={(id) =>
            navigateTo({
              view: "album",
              albumId: id
            })
          }
          onPlayAlbumById={handlePlayAlbumById}
          onPlayAlbumTrack={handlePlayAlbumTrack}
          onPause={handlePause}
          selectedAlbum={selectedAlbum}
          albumTracks={albumTracks}
          albumTracksLoading={albumTracksLoading}
          albumTracksError={albumTracksError}
          formatMs={formatMs}
          effectiveNowPlayingTrackId={effectiveNowPlayingTrackId}
          trackMenuTrackId={trackMenuTrackId}
          trackMenuPosition={trackMenuPosition}
          onToggleMenu={toggleTrackMenu}
          onMenuPlay={(trackId) =>
            runTrackMenuAction((id) => {
              handlePlay(id);
            }, trackId)
          }
          onMenuQueue={(trackId) =>
            runTrackMenuAction((id) => {
              handleQueue(id);
            }, trackId)
          }
          onMenuPlayNext={(trackId) =>
            runTrackMenuAction((id) => {
              handlePlayNext(id);
            }, trackId)
          }
          onMenuRescan={(trackId) =>
            runTrackMenuAction((id) => {
              handleRescanTrack(id);
            }, trackId)
          }
          onFixTrackMatch={(trackId) => runTrackMenuAction(openTrackMatchForAlbum, trackId)}
          onEditTrackMetadata={(trackId) =>
            runTrackMenuAction(openTrackEditorForAlbum, trackId)
          }
          onAnalyzeTrack={(track) => {
            runTrackMenuAction(() => {
              setAnalysisTarget({
                trackId: track.id,
                title: track.title ?? track.file_name,
                artist: track.artist ?? null
              });
            }, track.id);
          }}
          onEditAlbumMetadata={openAlbumEditor}
          onEditCatalogMetadata={() => setCatalogOpen(true)}
          onReadAlbumNotes={() => setAlbumNotesOpen(true)}
          albumProfile={albumProfile}
          settingsSection={settingsSection}
          onSettingsSectionChange={(section) =>
            navigateTo({
              view: "settings",
              settingsSection: section
            })
          }
          apiBase={apiBaseOverride}
          apiBaseDefault={apiBaseDefault}
          onApiBaseChange={handleApiBaseChange}
          onApiBaseReset={handleApiBaseReset}
          onReconnect={() => window.location.reload()}
          outputsSettings={outputsSettings}
          outputsProviders={outputsProviders}
          outputsLoading={outputsLoading}
          outputsError={outputsError}
          outputsLastRefresh={outputsLastRefresh}
          onRefreshProvider={handleRefreshProvider}
          onToggleOutput={handleToggleOutputSetting}
          onRenameOutput={handleRenameOutputSetting}
          onToggleExclusive={handleToggleExclusiveSetting}
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
      }
    >
      <AppModals
        showGate={showGate}
        isLocalSession={Boolean(isLocalSession)}
        createSessionOpen={createSessionOpen}
        createSessionBusy={createSessionBusy}
        newSessionName={newSessionName}
        newSessionNeverExpires={newSessionNeverExpires}
        onSetCreateSessionOpen={setCreateSessionOpen}
        onSetNewSessionName={setNewSessionName}
        onSetNewSessionNeverExpires={setNewSessionNeverExpires}
        onSubmitCreateSession={() => {
          void submitCreateSession();
        }}
        outputsOpen={outputsOpen}
        outputs={outputs}
        sessions={sessions}
        sessionOutputLocks={sessionOutputLocks}
        sessionBridgeLocks={sessionBridgeLocks}
        sessionId={sessionId}
        activeOutputId={activeOutputId}
        onSetOutputsOpen={setOutputsOpen}
        onSelectOutputForSession={handleSelectOutputForSession}
        formatRateRange={formatRateRange}
        signalOpen={signalOpen}
        status={status}
        activeOutput={activeOutput}
        updatedAt={updatedAt}
        formatHz={formatHz}
        onSetSignalOpen={setSignalOpen}
        matchOpen={Boolean(matchTarget)}
        matchLabel={matchLabel}
        matchDefaults={matchDefaults}
        matchTrackId={matchTarget?.trackId ?? null}
        onCloseMatch={() => setMatchTarget(null)}
        editOpen={Boolean(editTarget)}
        editTrackId={editTarget?.trackId ?? null}
        editLabel={editLabel}
        editDefaults={editDefaults}
        onCloseEdit={() => setEditTarget(null)}
        onSavedEdit={() => {
          if (albumViewId !== null) {
            loadAlbumTracks(albumViewId);
          }
          loadAlbums();
        }}
        albumEditOpen={Boolean(albumEditTarget)}
        albumEditAlbumId={albumEditTarget?.albumId ?? null}
        albumEditLabel={albumEditLabel}
        albumEditArtist={albumEditTarget?.artist ?? ""}
        albumEditDefaults={albumEditDefaults}
        nowPlayingAlbumId={nowPlayingAlbumId}
        isPlaying={isPlaying}
        onPause={handlePause}
        onCloseAlbumEdit={() => setAlbumEditTarget(null)}
        onUpdatedAlbumEdit={(updatedAlbumId) => {
          if (albumViewId !== null) {
            setAlbumViewId(updatedAlbumId);
            loadAlbumTracks(updatedAlbumId);
          }
          loadAlbums();
        }}
        albumNotesOpen={albumNotesOpen}
        selectedAlbumTitle={selectedAlbum?.title ?? ""}
        selectedAlbumArtist={selectedAlbum?.artist ?? ""}
        albumNotes={albumProfile?.notes?.text ?? ""}
        onCloseAlbumNotes={() => setAlbumNotesOpen(false)}
        analysisOpen={Boolean(analysisTarget)}
        analysisTrackId={analysisTarget?.trackId ?? null}
        analysisTitle={analysisTarget?.title ?? ""}
        analysisArtist={analysisTarget?.artist ?? null}
        onCloseAnalysis={() => setAnalysisTarget(null)}
        catalogOpen={catalogOpen}
        albumViewId={albumViewId}
        onCloseCatalog={() => setCatalogOpen(false)}
        onCatalogUpdated={({ album }) => {
          if (album) {
            setAlbumProfile(album);
          } else {
            loadCatalogProfiles(albumViewId);
          }
        }}
        queueOpen={queueOpen}
        queue={queue}
        formatMs={formatMs}
        placeholder={albumPlaceholder}
        canQueuePlay={Boolean(sessionId && (activeOutputId || isLocalSession))}
        isPaused={isPaused}
        onQueueClose={() => setQueueOpen(false)}
        onQueuePause={handlePause}
        onQueuePlayFrom={handleQueuePlayFrom}
        onQueueClear={handleQueueClear}
        audioRef={audioRef}
      />
    </AppChrome>
  );
}
