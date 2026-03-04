import AlbumDetailView from "./AlbumDetailView";
import AlbumsView from "./AlbumsView";
import NowPlayingScreen from "./NowPlayingScreen";
import QueueScreen from "./QueueScreen";
import SessionsScreen from "./SessionsScreen";
import SettingsView from "./SettingsView";
import { SettingsSection } from "../hooks/useViewNavigation";
import {
  AlbumProfileResponse,
  AlbumSummary,
  LogEvent,
  MetadataEvent,
  OutputSettings,
  ProviderOutputs,
  QueueItem,
  SessionSummary,
  StatusResponse,
  TrackSummary
} from "../types";

type MetadataEventEntry = {
  id: number;
  time: Date;
  event: MetadataEvent;
};

type LogEventEntry = {
  id: number;
  event: LogEvent;
};

type MainContentProps = {
  settingsOpen: boolean;
  queueViewOpen: boolean;
  nowPlayingViewOpen: boolean;
  sessionsViewOpen: boolean;
  albumViewId: number | null;
  filteredAlbums: AlbumSummary[];
  albumsLoading: boolean;
  albumsError: string | null;
  placeholder: (title?: string | null, artist?: string | null) => string;
  sessionId: string | null;
  activeOutputId: string | null;
  isLocalSession: boolean;
  activeAlbumId: number | null;
  isPlaying: boolean;
  isPaused: boolean;
  albumViewMode: "grid" | "list";
  onSelectAlbum: (id: number) => void;
  onPlayAlbumById: (albumId: number) => void;
  onPlayAlbumTrack: (track: TrackSummary) => void;
  onPause: () => void | Promise<void>;
  queue: QueueItem[];
  canQueuePlay: boolean;
  onQueuePause: () => void | Promise<void>;
  onQueuePlayFrom: (trackId: number) => void | Promise<void>;
  onQueueClear: (clearQueue: boolean, clearHistory: boolean) => void | Promise<void>;
  status: StatusResponse | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  hasNowPlaying: boolean;
  queueHasNext: boolean;
  onPrimaryAction: () => void | Promise<void>;
  onPrevious: () => void | Promise<void>;
  onNext: () => void | Promise<void>;
  onCoverError: () => void;
  sessions: SessionSummary[];
  serverConnected: boolean;
  onSessionChange: (nextSessionId: string) => void;
  onCreateSession: () => void;
  onDeleteSession: () => void;
  deleteSessionDisabled: boolean;
  selectedAlbum: AlbumSummary | null;
  albumTracks: TrackSummary[];
  albumTracksLoading: boolean;
  albumTracksError: string | null;
  formatMs: (ms?: number | null) => string;
  effectiveNowPlayingTrackId: number | null;
  trackMenuTrackId: number | null;
  trackMenuPosition: { top: number; right: number; up: boolean } | null;
  onToggleMenu: (trackId: number, target: Element) => void;
  onMenuPlay: (trackId: number) => void;
  onMenuQueue: (trackId: number) => void;
  onMenuPlayNext: (trackId: number) => void;
  onMenuRescan: (trackId: number) => void;
  onFixTrackMatch: (trackId: number) => void;
  onEditTrackMetadata: (trackId: number) => void;
  onAnalyzeTrack: (track: TrackSummary) => void;
  onEditAlbumMetadata: () => void;
  onEditCatalogMetadata: () => void;
  onReadAlbumNotes: () => void;
  albumProfile: AlbumProfileResponse | null;
  settingsSection: SettingsSection;
  onSettingsSectionChange: (section: SettingsSection) => void;
  apiBase: string;
  apiBaseDefault: string;
  onApiBaseChange: (value: string) => void;
  onApiBaseReset: () => void;
  onReconnect: () => void;
  outputsSettings: OutputSettings | null;
  outputsProviders: ProviderOutputs[];
  outputsLoading: boolean;
  outputsError: string | null;
  outputsLastRefresh: Record<string, string>;
  onRefreshProvider: (providerId: string) => void;
  onToggleOutput: (outputId: string, enabled: boolean) => void;
  onRenameOutput: (outputId: string, name: string) => void;
  onToggleExclusive: (outputId: string, enabled: boolean) => void;
  metadataEvents: MetadataEventEntry[];
  logEvents: LogEventEntry[];
  logsError: string | null;
  rescanBusy: boolean;
  onClearMetadata: () => void;
  onRescanLibrary: () => void;
  onClearLogs: () => void;
  describeMetadataEvent: (event: MetadataEvent) => { title: string; detail?: string };
  metadataDetailLines: (event: MetadataEvent) => string[];
};

export default function MainContent({
  settingsOpen,
  queueViewOpen,
  nowPlayingViewOpen,
  sessionsViewOpen,
  albumViewId,
  filteredAlbums,
  albumsLoading,
  albumsError,
  placeholder,
  sessionId,
  activeOutputId,
  isLocalSession,
  activeAlbumId,
  isPlaying,
  isPaused,
  albumViewMode,
  onSelectAlbum,
  onPlayAlbumById,
  onPlayAlbumTrack,
  onPause,
  queue,
  canQueuePlay,
  onQueuePause,
  onQueuePlayFrom,
  onQueueClear,
  status,
  nowPlayingCover,
  nowPlayingCoverFailed,
  canTogglePlayback,
  canGoPrevious,
  hasNowPlaying,
  queueHasNext,
  onPrimaryAction,
  onPrevious,
  onNext,
  onCoverError,
  sessions,
  serverConnected,
  onSessionChange,
  onCreateSession,
  onDeleteSession,
  deleteSessionDisabled,
  selectedAlbum,
  albumTracks,
  albumTracksLoading,
  albumTracksError,
  formatMs,
  effectiveNowPlayingTrackId,
  trackMenuTrackId,
  trackMenuPosition,
  onToggleMenu,
  onMenuPlay,
  onMenuQueue,
  onMenuPlayNext,
  onMenuRescan,
  onFixTrackMatch,
  onEditTrackMetadata,
  onAnalyzeTrack,
  onEditAlbumMetadata,
  onEditCatalogMetadata,
  onReadAlbumNotes,
  albumProfile,
  settingsSection,
  onSettingsSectionChange,
  apiBase,
  apiBaseDefault,
  onApiBaseChange,
  onApiBaseReset,
  onReconnect,
  outputsSettings,
  outputsProviders,
  outputsLoading,
  outputsError,
  outputsLastRefresh,
  onRefreshProvider,
  onToggleOutput,
  onRenameOutput,
  onToggleExclusive,
  metadataEvents,
  logEvents,
  logsError,
  rescanBusy,
  onClearMetadata,
  onRescanLibrary,
  onClearLogs,
  describeMetadataEvent,
  metadataDetailLines
}: MainContentProps) {
  return (
    <>
      {nowPlayingViewOpen && !settingsOpen ? (
        <NowPlayingScreen
          status={status}
          nowPlayingCover={nowPlayingCover}
          nowPlayingCoverFailed={nowPlayingCoverFailed}
          placeholderCover={placeholder(status?.album, status?.artist)}
          formatMs={formatMs}
          canTogglePlayback={canTogglePlayback}
          canGoPrevious={canGoPrevious}
          hasNowPlaying={hasNowPlaying}
          isPaused={isPaused}
          queueHasItems={queueHasNext}
          onCoverError={onCoverError}
          onPrimaryAction={onPrimaryAction}
          onPrevious={onPrevious}
          onNext={onNext}
        />
      ) : null}

      {sessionsViewOpen && !settingsOpen ? (
        <SessionsScreen
          sessionId={sessionId}
          sessions={sessions}
          serverConnected={serverConnected}
          onSessionChange={onSessionChange}
          onCreateSession={onCreateSession}
          onDeleteSession={onDeleteSession}
          deleteSessionDisabled={deleteSessionDisabled}
        />
      ) : null}

      {!settingsOpen && !queueViewOpen && !nowPlayingViewOpen && !sessionsViewOpen && albumViewId === null ? (
        <section className="grid">
          <AlbumsView
            albums={filteredAlbums}
            loading={albumsLoading}
            error={albumsError}
            placeholder={placeholder}
            canPlay={Boolean(sessionId && (activeOutputId || isLocalSession))}
            activeAlbumId={activeAlbumId}
            isPlaying={isPlaying}
            isPaused={isPaused}
            viewMode={albumViewMode}
            onSelectAlbum={onSelectAlbum}
            onPlayAlbum={onPlayAlbumById}
            onPause={onPause}
          />
        </section>
      ) : null}

      {queueViewOpen && !settingsOpen && !nowPlayingViewOpen && !sessionsViewOpen ? (
        <QueueScreen
          items={queue}
          formatMs={formatMs}
          placeholder={placeholder}
          canPlay={canQueuePlay}
          isPaused={isPaused}
          onPause={onQueuePause}
          onPlayFrom={onQueuePlayFrom}
          onClear={onQueueClear}
        />
      ) : null}

      {albumViewId !== null && !settingsOpen && !queueViewOpen && !nowPlayingViewOpen && !sessionsViewOpen ? (
        <AlbumDetailView
          album={selectedAlbum}
          tracks={albumTracks}
          loading={albumTracksLoading}
          error={albumTracksError}
          placeholder={placeholder}
          canPlay={Boolean(sessionId && (activeOutputId || isLocalSession)) && albumTracks.length > 0}
          activeAlbumId={activeAlbumId}
          isPlaying={isPlaying}
          isPaused={isPaused}
          onPause={onPause}
          formatMs={formatMs}
          nowPlayingTrackId={effectiveNowPlayingTrackId}
          onPlayAlbum={() => {
            if (!selectedAlbum) return;
            onPlayAlbumById(selectedAlbum.id);
          }}
          onPlayTrack={onPlayAlbumTrack}
          trackMenuTrackId={trackMenuTrackId}
          trackMenuPosition={trackMenuPosition}
          onToggleMenu={onToggleMenu}
          onMenuPlay={onMenuPlay}
          onMenuQueue={onMenuQueue}
          onMenuPlayNext={onMenuPlayNext}
          onMenuRescan={onMenuRescan}
          onFixTrackMatch={onFixTrackMatch}
          onEditTrackMetadata={onEditTrackMetadata}
          onAnalyzeTrack={onAnalyzeTrack}
          onEditAlbumMetadata={onEditAlbumMetadata}
          onEditCatalogMetadata={onEditCatalogMetadata}
          onReadAlbumNotes={onReadAlbumNotes}
          albumProfile={albumProfile}
        />
      ) : null}

      <SettingsView
        active={settingsOpen}
        section={settingsSection}
        onSectionChange={onSettingsSectionChange}
        apiBase={apiBase}
        apiBaseDefault={apiBaseDefault}
        onApiBaseChange={onApiBaseChange}
        onApiBaseReset={onApiBaseReset}
        onReconnect={onReconnect}
        outputsSettings={outputsSettings}
        outputsProviders={outputsProviders}
        outputsLoading={outputsLoading}
        outputsError={outputsError}
        outputsLastRefresh={outputsLastRefresh}
        onRefreshProvider={onRefreshProvider}
        onToggleOutput={onToggleOutput}
        onRenameOutput={onRenameOutput}
        onToggleExclusive={onToggleExclusive}
        metadataEvents={metadataEvents}
        logEvents={logEvents}
        logsError={logsError}
        rescanBusy={rescanBusy}
        onClearMetadata={onClearMetadata}
        onRescanLibrary={onRescanLibrary}
        onClearLogs={onClearLogs}
        describeMetadataEvent={describeMetadataEvent}
        metadataDetailLines={metadataDetailLines}
      />
    </>
  );
}
