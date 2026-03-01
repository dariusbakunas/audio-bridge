import { Library, PanelLeftClose, PanelLeftOpen, Settings } from "lucide-react";

import { ViewState } from "../hooks/useViewNavigation";

type SideNavProps = {
  navCollapsed: boolean;
  settingsOpen: boolean;
  onToggleCollapsed: () => void;
  navigateTo: (next: ViewState) => void;
};

export default function SideNav({
  navCollapsed,
  settingsOpen,
  onToggleCollapsed,
  navigateTo
}: SideNavProps) {
  return (
    <aside className="side-nav">
      <div className="nav-brand">
        <div className="nav-brand-text">
          <div className="nav-title">Audio Hub</div>
          <div className="nav-subtitle">Lossless control with a live signal view.</div>
        </div>
        <button
          className="icon-btn nav-collapse"
          onClick={onToggleCollapsed}
          aria-label={navCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          title={navCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          type="button"
        >
          {navCollapsed ? (
            <PanelLeftOpen className="icon" aria-hidden="true" />
          ) : (
            <PanelLeftClose className="icon" aria-hidden="true" />
          )}
        </button>
      </div>
      <div className="nav-section">
        <div className="nav-label">Library</div>
        <button
          className={`nav-button ${!settingsOpen ? "active" : ""}`}
          onClick={() =>
            navigateTo({
              view: "albums"
            })
          }
        >
          <Library className="nav-icon" aria-hidden="true" />
          <span>Albums</span>
        </button>
      </div>
      <div className="nav-section">
        <div className="nav-label">System</div>
        <button
          className={`nav-button ${settingsOpen ? "active" : ""}`}
          onClick={() =>
            navigateTo({
              view: "settings",
              settingsSection: "metadata"
            })
          }
        >
          <Settings className="nav-icon" aria-hidden="true" />
          <span>Settings</span>
        </button>
      </div>
    </aside>
  );
}
