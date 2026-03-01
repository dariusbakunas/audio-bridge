import { ReactNode } from "react";

import { OutputInfo, SessionSummary, SessionVolumeResponse, StatusResponse } from "../types";
import { SettingsSection, ViewState } from "../hooks/useViewNavigation";
import ConnectionGate from "./ConnectionGate";
import MainContent from "./MainContent";
import NotificationsPanel from "./NotificationsPanel";
import PlayerBar from "./PlayerBar";
import SideNav from "./SideNav";
import ViewHeader from "./ViewHeader";

type AppChromeProps = {
  settingsOpen: boolean;
  showGate: boolean;
  navCollapsed: boolean;
  onToggleNavCollapsed: () => void;
  navigateTo: (next: ViewState) => void;
  serverConnecting: boolean;
  serverError: string | null;
  apiBaseOverride: string;
  apiBaseDefault: string;
  onApiBaseChange: (value: string) => void;
  onApiBaseReset: () => void;
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  viewTitle: string;
  albumViewId: number | null;
  albumSearch: string;
  onAlbumSearchChange: (value: string) => void;
  albumViewMode: "grid" | "list";
  onAlbumViewModeChange: (mode: "grid" | "list") => void;
  sessionId: string | null;
  sessions: SessionSummary[];
  serverConnected: boolean;
  onSessionChange: (nextSessionId: string) => void;
  onCreateSession: () => void;
  onDeleteSession: () => void;
  deleteSessionDisabled: boolean;
  notificationsOpen: boolean;
  unreadCount: number;
  notifications: { id: number; level: "error" | "warn" | "info" | "success"; message: string; createdAt: Date }[];
  onToggleNotifications: () => void;
  onClearNotifications: () => void;
  playerVisible: boolean;
  playerStatus: StatusResponse | null;
  playerUpdatedAt: Date | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  isLocalSession: boolean;
  hasNowPlaying: boolean;
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  isPaused: boolean;
  playButtonTitle?: string;
  queueHasNext: boolean;
  queueOpen: boolean;
  sessionVolume: SessionVolumeResponse | null;
  volumeBusy: boolean;
  activeOutput: OutputInfo | null;
  activeAlbumId: number | null;
  uiBuildId: string;
  formatMs: (ms?: number | null) => string;
  albumPlaceholder: (title?: string | null, artist?: string | null) => string;
  onCoverError: () => void;
  onAlbumNavigate: (albumId: number) => void;
  onPrimaryAction: () => void | Promise<void>;
  onPrevious: () => void | Promise<void>;
  onNext: () => void | Promise<void>;
  onSignalOpen: () => void;
  onQueueOpen: () => void;
  onVolumeChange: (value: number) => void | Promise<void>;
  onVolumeToggleMute: () => void | Promise<void>;
  onSelectOutput: () => void;
  mainContent: ReactNode;
  children?: ReactNode;
};

export default function AppChrome({
  settingsOpen,
  showGate,
  navCollapsed,
  onToggleNavCollapsed,
  navigateTo,
  serverConnecting,
  serverError,
  apiBaseOverride,
  apiBaseDefault,
  onApiBaseChange,
  onApiBaseReset,
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  viewTitle,
  albumViewId,
  albumSearch,
  onAlbumSearchChange,
  albumViewMode,
  onAlbumViewModeChange,
  sessionId,
  sessions,
  serverConnected,
  onSessionChange,
  onCreateSession,
  onDeleteSession,
  deleteSessionDisabled,
  notificationsOpen,
  unreadCount,
  notifications,
  onToggleNotifications,
  onClearNotifications,
  playerVisible,
  playerStatus,
  playerUpdatedAt,
  nowPlayingCover,
  nowPlayingCoverFailed,
  isLocalSession,
  hasNowPlaying,
  canTogglePlayback,
  canGoPrevious,
  isPaused,
  playButtonTitle,
  queueHasNext,
  queueOpen,
  sessionVolume,
  volumeBusy,
  activeOutput,
  activeAlbumId,
  uiBuildId,
  formatMs,
  albumPlaceholder,
  onCoverError,
  onAlbumNavigate,
  onPrimaryAction,
  onPrevious,
  onNext,
  onSignalOpen,
  onQueueOpen,
  onVolumeChange,
  onVolumeToggleMute,
  onSelectOutput,
  mainContent,
  children
}: AppChromeProps) {
  return (
    <div className={`app ${settingsOpen ? "settings-mode" : ""} ${showGate ? "has-gate" : ""}`}>
      {showGate ? (
        <ConnectionGate
          status={serverConnecting ? "connecting" : "disconnected"}
          message={serverError}
          apiBase={apiBaseOverride}
          apiBaseDefault={apiBaseDefault}
          onApiBaseChange={onApiBaseChange}
          onApiBaseReset={onApiBaseReset}
          onReconnect={() => window.location.reload()}
        />
      ) : null}
      <div className={`layout ${navCollapsed ? "nav-collapsed" : ""}`}>
        <SideNav
          navCollapsed={navCollapsed}
          settingsOpen={settingsOpen}
          onToggleCollapsed={onToggleNavCollapsed}
          navigateTo={navigateTo}
        />

        <main className={`main ${showGate ? "disabled" : ""}`}>
          <ViewHeader
            canGoBack={canGoBack}
            canGoForward={canGoForward}
            onGoBack={onGoBack}
            onGoForward={onGoForward}
            viewTitle={viewTitle}
            showLibraryTools={!settingsOpen && albumViewId === null}
            albumSearch={albumSearch}
            onAlbumSearchChange={onAlbumSearchChange}
            albumViewMode={albumViewMode}
            onAlbumViewModeChange={onAlbumViewModeChange}
            sessionId={sessionId}
            sessions={sessions}
            serverConnected={serverConnected}
            onSessionChange={onSessionChange}
            onCreateSession={onCreateSession}
            onDeleteSession={onDeleteSession}
            deleteSessionDisabled={deleteSessionDisabled}
            notificationsOpen={notificationsOpen}
            unreadCount={unreadCount}
            onToggleNotifications={onToggleNotifications}
          />

          {mainContent}
        </main>
      </div>

      <NotificationsPanel
        open={notificationsOpen}
        showGate={showGate}
        notifications={notifications}
        onClose={onToggleNotifications}
        onClear={onClearNotifications}
      />

      {playerVisible ? (
        <PlayerBar
          status={playerStatus}
          updatedAt={playerUpdatedAt}
          nowPlayingCover={nowPlayingCover}
          nowPlayingCoverFailed={nowPlayingCoverFailed}
          showSignalAction={!isLocalSession}
          showSignalPath={hasNowPlaying}
          canTogglePlayback={canTogglePlayback}
          canGoPrevious={canGoPrevious}
          hasNowPlaying={hasNowPlaying}
          isPaused={isPaused}
          playButtonTitle={playButtonTitle}
          queueHasItems={queueHasNext}
          queueOpen={queueOpen}
          volume={sessionVolume}
          volumeBusy={volumeBusy}
          showOutputAction={!isLocalSession}
          activeOutput={activeOutput}
          activeAlbumId={activeAlbumId}
          uiBuildId={uiBuildId}
          formatMs={formatMs}
          placeholderCover={albumPlaceholder(playerStatus?.album, playerStatus?.artist)}
          onCoverError={onCoverError}
          onAlbumNavigate={onAlbumNavigate}
          onPrimaryAction={() => {
            void onPrimaryAction();
          }}
          onPrevious={() => {
            void onPrevious();
          }}
          onNext={() => {
            void onNext();
          }}
          onSignalOpen={onSignalOpen}
          onQueueOpen={onQueueOpen}
          onVolumeChange={(value) => {
            void onVolumeChange(value);
          }}
          onVolumeToggleMute={() => {
            void onVolumeToggleMute();
          }}
          onSelectOutput={onSelectOutput}
        />
      ) : null}

      {children}
    </div>
  );
}
