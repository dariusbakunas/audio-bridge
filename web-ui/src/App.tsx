import { useCallback, useEffect, useMemo, useRef, SetStateAction, useState } from "react";
import {
  fetchJson,
  postJson
} from "./api";
import {
  LogEvent,
  MetadataEvent,
  OutputInfo,
  SessionVolumeResponse,
  StatusResponse,
  QueueItem
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
import { useAlbumMetadataTargets } from "./hooks/useAlbumMetadataTargets";
import { useAlbumsState } from "./hooks/useAlbumsState";
import { useLocalPlayback } from "./hooks/useLocalPlayback";
import { useMediaSessionControls } from "./hooks/useMediaSessionControls";
import { useNowPlayingCover } from "./hooks/useNowPlayingCover";
import { useOutputSettings } from "./hooks/useOutputSettings";
import { usePlaybackCommands } from "./hooks/usePlaybackCommands";
import { useSessionUiActions } from "./hooks/useSessionUiActions";
import { useSessionsState } from "./hooks/useSessionsState";
import { useTrackMenu } from "./hooks/useTrackMenu";
import { useToasts } from "./hooks/useToasts";
import { SettingsSection, useViewNavigation } from "./hooks/useViewNavigation";
import {
  albumPlaceholder,
  describeMetadataEvent,
  formatHz,
  formatMs,
  formatRateRange,
  metadataDetailLines,
  normalizeMatch
} from "./utils/viewFormatters";

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
const WEB_SESSION_CLIENT_ID_KEY = "audioHub.webSessionClientId";
const WEB_SESSION_ID_KEY = "audioHub.webSessionId";
const NAV_COLLAPSED_KEY = "audioHub.navCollapsed";
const WEB_DEFAULT_SESSION_NAME = "Default";

function isDefaultSessionName(name: string | null | undefined): boolean {
  return (name ?? "").trim().toLowerCase() === WEB_DEFAULT_SESSION_NAME.toLowerCase();
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
  const {
    matchTarget,
    setMatchTarget,
    editTarget,
    setEditTarget,
    albumEditTarget,
    setAlbumEditTarget,
    openTrackMatchForAlbum,
    openAlbumEditor,
    openTrackEditorForAlbum,
    matchLabel,
    matchDefaults,
    editLabel,
    editDefaults,
    albumEditLabel,
    albumEditDefaults
  } = useAlbumMetadataTargets({
    albumTracks,
    selectedAlbum
  });
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

  const resetSessionContext = useCallback(() => {
    setStatus(null);
    setSessionVolume(null);
    setQueue([]);
  }, []);

  const clearSessionSelection = useCallback(() => {
    setSessionId(null);
    setActiveOutputId(null);
    resetSessionContext();
  }, [resetSessionContext, setActiveOutputId, setSessionId]);

  const {
    createSessionOpen,
    setCreateSessionOpen,
    newSessionName,
    setNewSessionName,
    newSessionNeverExpires,
    setNewSessionNeverExpires,
    createSessionBusy,
    handleSessionChange,
    handleCreateSession,
    submitCreateSession,
    handleDeleteSession
  } = useSessionUiActions({
    sessions,
    sessionId,
    refreshSessions,
    refreshSessionLocks,
    selectSession,
    reportError,
    getClientId: getOrCreateWebSessionClientId,
    appVersion: __APP_VERSION__,
    sessionStorageKey: WEB_SESSION_ID_KEY,
    onSessionContextReset: resetSessionContext,
    onNoSessionsRemaining: clearSessionSelection,
    isDefaultSessionName
  });

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

  const {
    handlePause,
    handlePlay,
    handleNext,
    handlePrevious,
    handleQueuePlayFrom,
    handlePlayAlbumTrack,
    handlePlayAlbumById
  } = usePlaybackCommands({
    isLocalSession: Boolean(isLocalSession),
    sessionId,
    reportError,
    applyLocalPlayback,
    requestLocalCommand,
    toggleLocalPause,
    handlePauseRemote,
    handlePlayRemote,
    handlePlayAlbumTrackRemote,
    handlePlayAlbumByIdRemote,
    handleNextRemote,
    handlePreviousRemote,
    handleQueuePlayFromRemote
  });

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
  const { handlePrimaryAction } = useMediaSessionControls({
    status,
    nowPlayingCover,
    hasNowPlaying,
    replayTrackId,
    isLocalSession,
    sessionId,
    queue,
    effectiveNowPlayingTrackId,
    resumeLocalFromStatus,
    handlePause,
    handleQueuePlayFrom,
    handlePrevious,
    handleNext,
    reportError
  });

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
