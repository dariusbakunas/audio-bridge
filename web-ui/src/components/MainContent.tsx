import AlbumDetailView from "./AlbumDetailView";
import AlbumsView from "./AlbumsView";
import SettingsView from "./SettingsView";
import { SettingsSection } from "../hooks/useViewNavigation";
import {
  AlbumProfileResponse,
  AlbumSummary,
  LogEvent,
  MetadataEvent,
  OutputSettings,
  ProviderOutputs,
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
      {!settingsOpen && albumViewId === null ? (
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

      {albumViewId !== null && !settingsOpen ? (
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
