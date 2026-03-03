import { useCallback, useMemo, useRef } from "react";
import AppModals from "./components/AppModals";
import AppChrome from "./components/AppChrome";
import MainContent from "./components/MainContent";
import { useActivityEvents } from "./hooks/useActivityEvents";
import { useAppChromeActions } from "./hooks/useAppChromeActions";
import { useAppUiState } from "./hooks/useAppUiState";
import { useAlbumModalActions } from "./hooks/useAlbumModalActions";
import { usePlaybackActions } from "./hooks/usePlaybackActions";
import { useQueueActions } from "./hooks/useQueueActions";
import { useHubConnection } from "./hooks/useHubConnection";
import { useAlbumMetadataTargets } from "./hooks/useAlbumMetadataTargets";
import { useAlbumViewState } from "./hooks/useAlbumViewState";
import { useAlbumsState } from "./hooks/useAlbumsState";
import { useLocalPlayback } from "./hooks/useLocalPlayback";
import { useMainContentActions } from "./hooks/useMainContentActions";
import { useMediaSessionControls } from "./hooks/useMediaSessionControls";
import { useNowPlayingCover } from "./hooks/useNowPlayingCover";
import { useOutputSettings } from "./hooks/useOutputSettings";
import { usePlaybackCommands } from "./hooks/usePlaybackCommands";
import { usePlaybackDerivedState } from "./hooks/usePlaybackDerivedState";
import { useSessionUiActions } from "./hooks/useSessionUiActions";
import { useSessionContext } from "./hooks/useSessionContext";
import { useSessionStreams } from "./hooks/useSessionStreams";
import { useSessionVolumeControl } from "./hooks/useSessionVolumeControl";
import { useSessionOutputSelection } from "./hooks/useSessionOutputSelection";
import { useSessionsState } from "./hooks/useSessionsState";
import { useTrackMenu } from "./hooks/useTrackMenu";
import { useToasts } from "./hooks/useToasts";
import { useUiShellEffects } from "./hooks/useUiShellEffects";
import { useViewNavigation } from "./hooks/useViewNavigation";
import {
  albumPlaceholder,
  describeMetadataEvent,
  formatHz,
  formatMs,
  formatRateRange,
  metadataDetailLines
} from "./utils/viewFormatters";
import { getOrCreateWebSessionClientId, isDefaultSessionName } from "./utils/session";

const WEB_SESSION_CLIENT_ID_KEY = "audioHub.webSessionClientId";
const WEB_SESSION_ID_KEY = "audioHub.webSessionId";
const NAV_COLLAPSED_KEY = "audioHub.navCollapsed";

export default function App() {
  const {
    outputs,
    setOutputs,
    status,
    setStatus,
    queue,
    setQueue,
    rescanBusy,
    setRescanBusy,
    queueOpen,
    setQueueOpen,
    signalOpen,
    setSignalOpen,
    outputsOpen,
    setOutputsOpen,
    settingsOpen,
    setSettingsOpen,
    catalogOpen,
    setCatalogOpen,
    albumNotesOpen,
    setAlbumNotesOpen,
    analysisTarget,
    setAnalysisTarget,
    navCollapsed,
    setNavCollapsed,
    settingsSection,
    setSettingsSection,
    albumSearch,
    setAlbumSearch,
    albumViewMode,
    setAlbumViewMode,
    albumViewId,
    setAlbumViewId,
    updatedAt,
    setUpdatedAt
  } = useAppUiState({
    navCollapsedKey: NAV_COLLAPSED_KEY
  });
  const activeSessionIdRef = useRef<string | null>(null);
  const isLocalSessionRef = useRef<boolean>(false);
  const getClientId = useCallback(
    () => getOrCreateWebSessionClientId(WEB_SESSION_CLIENT_ID_KEY),
    []
  );
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
    getClientId,
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
  const { onSavedEdit, onUpdatedAlbumEdit, onCatalogUpdated } = useAlbumModalActions({
    albumViewId,
    setAlbumViewId,
    loadAlbumTracks,
    loadAlbums,
    setAlbumProfile,
    loadCatalogProfiles
  });
  const activeOutput = useMemo(
      () => outputs.find((output) => output.id === activeOutputId) ?? null,
      [outputs, activeOutputId]
  );
  const activeOutputAvailable = Boolean(activeOutput);
  const currentSession = useMemo(
    () => sessions.find((session) => session.id === sessionId) ?? null,
    [sessions, sessionId]
  );
  const isLocalSession = Boolean(currentSession?.mode === "local");
  const {
    audioRef,
    applyLocalPlayback,
    requestLocalCommand,
    toggleLocalPause,
    resumeLocalFromStatus
  } = useLocalPlayback({
    isLocalSession,
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
    isLocalSession,
    sessionId,
    activeOutputAvailable,
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
    isLocalSession,
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
    isLocalSession,
    statusNowPlayingTrackId: status?.now_playing_track_id,
    signalOpen,
    setSignalOpen,
    outputsOpen,
    setOutputsOpen,
    albumViewId,
    setAlbumNotesOpen
  });

  const { resetSessionContext, clearSessionSelection } = useSessionContext({
    setStatus,
    setSessionVolume,
    setQueue,
    setSessionId,
    setActiveOutputId
  });

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
    getClientId,
    appVersion: __APP_VERSION__,
    sessionStorageKey: WEB_SESSION_ID_KEY,
    onSessionContextReset: resetSessionContext,
    onNoSessionsRemaining: clearSessionSelection,
    isDefaultSessionName
  });

  useSessionStreams({
    serverConnected,
    streamKey,
    sessionId,
    activeOutputId,
    activeOutputAvailable,
    isLocalSession: Boolean(isLocalSession),
    activeSessionIdRef,
    isLocalSessionRef,
    setOutputs,
    setStatus,
    setQueue,
    setUpdatedAt,
    markServerConnected,
    refreshSessionDetail,
    connectionError,
    reportError
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
    isLocalSession,
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
  const {
    onSelectAlbum,
    onSettingsSectionChange,
    onMenuPlay,
    onMenuQueue,
    onMenuPlayNext,
    onMenuRescan,
    onFixTrackMatch,
    onEditTrackMetadata,
    onAnalyzeTrack
  } = useMainContentActions({
    navigateTo,
    runTrackMenuAction,
    handlePlay,
    handleQueue,
    handlePlayNext,
    handleRescanTrack,
    openTrackMatchForAlbum,
    openTrackEditorForAlbum,
    setAnalysisTarget
  });
  const handleSelectOutputForSession = useSessionOutputSelection({
    isLocalSession,
    sessionId,
    handleSelectOutput,
    refreshSessions,
    refreshSessionLocks,
    refreshSessionDetail
  });
  const { onAlbumNavigate, onSignalOpen, onQueueOpen, onSelectOutput, onDeleteSession } =
    useAppChromeActions({
      navigateTo,
      isLocalSession,
      setSignalOpen,
      setQueueOpen,
      setOutputsOpen,
      handleDeleteSession
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
      onDeleteSession={onDeleteSession}
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
      isLocalSession={isLocalSession}
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
      onAlbumNavigate={onAlbumNavigate}
      onPrimaryAction={handlePrimaryAction}
      onPrevious={handlePrevious}
      onNext={handleNext}
      onSignalOpen={onSignalOpen}
      onQueueOpen={onQueueOpen}
      onVolumeChange={handleSetVolume}
      onVolumeToggleMute={handleToggleMute}
      onSelectOutput={onSelectOutput}
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
          isLocalSession={isLocalSession}
          activeAlbumId={activeAlbumId}
          isPlaying={isPlaying}
          isPaused={isPaused}
          albumViewMode={albumViewMode}
          onSelectAlbum={onSelectAlbum}
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
          onMenuPlay={onMenuPlay}
          onMenuQueue={onMenuQueue}
          onMenuPlayNext={onMenuPlayNext}
          onMenuRescan={onMenuRescan}
          onFixTrackMatch={onFixTrackMatch}
          onEditTrackMetadata={onEditTrackMetadata}
          onAnalyzeTrack={onAnalyzeTrack}
          onEditAlbumMetadata={openAlbumEditor}
          onEditCatalogMetadata={() => setCatalogOpen(true)}
          onReadAlbumNotes={() => setAlbumNotesOpen(true)}
          albumProfile={albumProfile}
          settingsSection={settingsSection}
          onSettingsSectionChange={onSettingsSectionChange}
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
        isLocalSession={isLocalSession}
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
        onSavedEdit={onSavedEdit}
        albumEditOpen={Boolean(albumEditTarget)}
        albumEditAlbumId={albumEditTarget?.albumId ?? null}
        albumEditLabel={albumEditLabel}
        albumEditArtist={albumEditTarget?.artist ?? ""}
        albumEditDefaults={albumEditDefaults}
        nowPlayingAlbumId={nowPlayingAlbumId}
        isPlaying={isPlaying}
        onPause={handlePause}
        onCloseAlbumEdit={() => setAlbumEditTarget(null)}
        onUpdatedAlbumEdit={onUpdatedAlbumEdit}
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
        onCatalogUpdated={onCatalogUpdated}
        queueOpen={queueOpen}
        queue={queue}
        formatMs={formatMs}
        placeholder={albumPlaceholder}
        canQueuePlay={Boolean(sessionId && (activeOutputAvailable || isLocalSession))}
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
