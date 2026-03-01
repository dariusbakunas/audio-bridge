import { Bell, ChevronLeft, ChevronRight, Grid3x3, List, Radio, Search, Trash2 } from "lucide-react";

import { SessionSummary } from "../types";

type ViewHeaderProps = {
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  viewTitle: string;
  showLibraryTools: boolean;
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
  onToggleNotifications: () => void;
};

export default function ViewHeader({
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  viewTitle,
  showLibraryTools,
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
  onToggleNotifications
}: ViewHeaderProps) {
  return (
    <header className="view-header">
      <div className="view-header-row">
        <div className="view-nav">
          <button
            className="icon-btn"
            onClick={onGoBack}
            disabled={!canGoBack}
            aria-label="Back"
            title="Back"
            type="button"
          >
            <ChevronLeft className="icon" aria-hidden="true" />
          </button>
          {canGoForward ? (
            <button
              className="icon-btn"
              onClick={onGoForward}
              aria-label="Forward"
              title="Forward"
              type="button"
            >
              <ChevronRight className="icon" aria-hidden="true" />
            </button>
          ) : null}
        </div>
        {viewTitle ? <h1>{viewTitle}</h1> : <span />}
        <div className="view-header-actions">
          {showLibraryTools ? (
            <div className="header-tools">
              <div className="header-search">
                <Search className="icon" aria-hidden="true" />
                <input
                  className="header-search-input"
                  type="search"
                  placeholder="Search albums, artists..."
                  value={albumSearch}
                  onChange={(event) => onAlbumSearchChange(event.target.value)}
                  aria-label="Search albums"
                />
              </div>
              <div className="view-toggle" role="tablist" aria-label="Album view">
                <button
                  type="button"
                  className={`view-toggle-btn ${albumViewMode === "grid" ? "active" : ""}`}
                  onClick={() => onAlbumViewModeChange("grid")}
                  aria-pressed={albumViewMode === "grid"}
                  title="Grid view"
                >
                  <Grid3x3 className="icon" aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className={`view-toggle-btn ${albumViewMode === "list" ? "active" : ""}`}
                  onClick={() => onAlbumViewModeChange("list")}
                  aria-pressed={albumViewMode === "list"}
                  title="List view"
                >
                  <List className="icon" aria-hidden="true" />
                </button>
              </div>
            </div>
          ) : null}
          <div className="header-session">
            <select
              className="header-session-select"
              value={sessionId ?? ""}
              onChange={(event) => onSessionChange(event.target.value)}
              aria-label="Playback session"
              title="Playback session"
              disabled={!serverConnected || sessions.length === 0}
            >
              {sessions.length === 0 ? <option value="">No session</option> : null}
              {sessions.map((session) => (
                <option key={session.id} value={session.id}>
                  {session.name}
                </option>
              ))}
            </select>
            <button
              className="icon-btn"
              type="button"
              onClick={onCreateSession}
              title="Create new session"
              aria-label="Create new session"
              disabled={!serverConnected}
            >
              <Radio className="icon" aria-hidden="true" />
            </button>
            <button
              className="icon-btn"
              type="button"
              onClick={onDeleteSession}
              title="Delete selected session"
              aria-label="Delete selected session"
              disabled={deleteSessionDisabled}
            >
              <Trash2 className="icon" aria-hidden="true" />
            </button>
          </div>
          <button
            className={`icon-btn notification-btn ${notificationsOpen ? "active" : ""}`}
            onClick={onToggleNotifications}
            aria-label="Notifications"
            title="Notifications"
            type="button"
          >
            <Bell className="icon" aria-hidden="true" />
            {unreadCount > 0 ? (
              <span className="notification-badge">{unreadCount > 99 ? "99+" : unreadCount}</span>
            ) : null}
          </button>
        </div>
      </div>
    </header>
  );
}
