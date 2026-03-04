import { Library, ListMusic, Radio, Settings, Speaker } from "lucide-react";

import { ViewState } from "../hooks/useViewNavigation";

type MobileTabBarProps = {
  settingsOpen: boolean;
  queueViewOpen: boolean;
  nowPlayingViewOpen: boolean;
  sessionsViewOpen: boolean;
  navigateTo: (next: ViewState) => void;
};

export default function MobileTabBar({
  settingsOpen,
  queueViewOpen,
  nowPlayingViewOpen,
  sessionsViewOpen,
  navigateTo
}: MobileTabBarProps) {
  const libraryActive = !settingsOpen && !queueViewOpen && !nowPlayingViewOpen && !sessionsViewOpen;

  return (
    <nav className="mobile-tab-bar" aria-label="Mobile tabs">
      <button
        className={`mobile-tab ${nowPlayingViewOpen ? "active" : ""}`}
        type="button"
        onClick={() => navigateTo({ view: "nowPlaying" })}
      >
        <Speaker className="icon" aria-hidden="true" />
        <span>Now Playing</span>
      </button>
      <button
        className={`mobile-tab ${libraryActive ? "active" : ""}`}
        type="button"
        onClick={() => navigateTo({ view: "albums" })}
      >
        <Library className="icon" aria-hidden="true" />
        <span>Library</span>
      </button>
      <button
        className={`mobile-tab ${sessionsViewOpen ? "active" : ""}`}
        type="button"
        onClick={() => navigateTo({ view: "sessions" })}
      >
        <Radio className="icon" aria-hidden="true" />
        <span>Sessions</span>
      </button>
      <button
        className={`mobile-tab ${queueViewOpen ? "active" : ""}`}
        type="button"
        onClick={() => navigateTo({ view: "queue" })}
      >
        <ListMusic className="icon" aria-hidden="true" />
        <span>Queue</span>
      </button>
      <button
        className={`mobile-tab ${settingsOpen ? "active" : ""}`}
        type="button"
        onClick={() =>
          navigateTo({
            view: "settings",
            settingsSection: "metadata"
          })
        }
      >
        <Settings className="icon" aria-hidden="true" />
        <span>Settings</span>
      </button>
    </nav>
  );
}
