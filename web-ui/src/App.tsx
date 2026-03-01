import { useCallback, useMemo, useRef, SetStateAction, useState } from "react";
import {
  OutputInfo,
  StatusResponse,
  QueueItem
} from "./types";
import AppModals from "./components/AppModals";
import AppChrome from "./components/AppChrome";
import MainContent from "./components/MainContent";
import {
  useOutputsStream,
  useQueueStream,
  useStatusStream
} from "./hooks/streams";
import { useActivityEvents } from "./hooks/useActivityEvents";
import { usePlaybackActions } from "./hooks/usePlaybackActions";
import { useQueueActions } from "./hooks/useQueueActions";
import { useHubConnection } from "./hooks/useHubConnection";
import { useAlbumMetadataTargets } from "./hooks/useAlbumMetadataTargets";
import { useAlbumViewState } from "./hooks/useAlbumViewState";
import { useAlbumsState } from "./hooks/useAlbumsState";
import { useLocalPlayback } from "./hooks/useLocalPlayback";
import { useMediaSessionControls } from "./hooks/useMediaSessionControls";
import { useNowPlayingCover } from "./hooks/useNowPlayingCover";
import { useOutputSettings } from "./hooks/useOutputSettings";
import { usePlaybackCommands } from "./hooks/usePlaybackCommands";
import { usePlaybackDerivedState } from "./hooks/usePlaybackDerivedState";
import { useSessionUiActions } from "./hooks/useSessionUiActions";
import { useSessionVolumeControl } from "./hooks/useSessionVolumeControl";
import { useSessionsState } from "./hooks/useSessionsState";
import { useTrackMenu } from "./hooks/useTrackMenu";
import { useToasts } from "./hooks/useToasts";
import { useUiShellEffects } from "./hooks/useUiShellEffects";
import { SettingsSection, useViewNavigation } from "./hooks/useViewNavigation";
import {
  albumPlaceholder,
  describeMetadataEvent,
  formatHz,
  formatMs,
  formatRateRange,
  metadataDetailLines
} from "./utils/viewFormatters";

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
  const [albumSearch, setAlbumSearch] = useState<string>("");
  const [albumViewMode, setAlbumViewMode] = useState<"grid" | "list">("grid");
  const [albumViewId, setAlbumViewId] = useState<number | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);
  const isLocalSessionRef = useRef<boolean>(false);
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
    metadataEvents,
    setMetadataEvents,
    logEvents,
    logsError,
    handleClearLogs
  } = useActivityEvents({
    settingsOpen,
    serverConnected,
    settingsSection,
    connectionError,
    reportError
  });
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
  const {
    replayTrackId,
    effectiveNowPlayingTrackId,
    hasNowPlaying,
    canReplayFromHistory,
    canTogglePlayback,
    canControlVolume,
    isPlaying,
    isPaused,
    viewTitle,
    playButtonTitle,
    showGate,
    queueHasNext,
    deleteSessionDisabled,
    canGoPrevious
  } = usePlaybackDerivedState({
    queue,
    status,
    isLocalSession: Boolean(isLocalSession),
    sessionId,
    activeOutputId,
    serverConnected,
    settingsOpen,
    albumViewId,
    sessions,
    isDefaultSessionName
  });
  const uiBuildId = useMemo(() => {
    if (__BUILD_MODE__ === "development") {
      return "dev";
    }
    return `v${__APP_VERSION__}+${__GIT_SHA__}`;
  }, []);

  const { selectedAlbum, filteredAlbums, activeAlbumId } = useAlbumViewState({
    albums,
    albumViewId,
    albumSearch,
    statusAlbum: status?.album,
    statusArtist: status?.artist,
    nowPlayingAlbumId
  });
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
  const {
    sessionVolume,
    setSessionVolume,
    volumeBusy,
    handleSetVolume,
    handleToggleMute
  } = useSessionVolumeControl({
    sessionId,
    isLocalSession: Boolean(isLocalSession),
    activeOutputId,
    canControlVolume,
    reportError
  });

  useUiShellEffects({
    navCollapsed,
    navCollapsedKey: NAV_COLLAPSED_KEY,
    serverConnected,
    setAlbumsError,
    setAlbumTracksError,
    activeSessionIdRef,
    sessionId,
    isLocalSessionRef,
    isLocalSession: Boolean(isLocalSession),
    statusNowPlayingTrackId: status?.now_playing_track_id,
    signalOpen,
    setSignalOpen,
    outputsOpen,
    setOutputsOpen,
    albumViewId,
    setAlbumNotesOpen
  });

  const resetSessionContext = useCallback(() => {
    setStatus(null);
    setSessionVolume(null);
    setQueue([]);
  }, [setSessionVolume]);

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
