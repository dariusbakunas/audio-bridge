import { LibraryEntry } from "../types";
import LibraryList from "./LibraryList";

interface FoldersViewProps {
  entries: LibraryEntry[];
  dir: string | null;
  loading: boolean;
  selectedTrackPath: string | null;
  trackMenuPath: string | null;
  trackMenuPosition: { top: number; right: number } | null;
  canPlay: boolean;
  formatMs: (ms?: number | null) => string;
  onRescan: () => void;
  onNavigateUp: () => void;
  onBackToRoot: () => void;
  onSelectDir: (dir: string | null) => void;
  onSelectTrack: (path: string | null) => void;
  onToggleMenu: (path: string, target: Element) => void;
  onPlay: (path: string) => void;
  onQueue: (path: string) => void;
  onPlayNext: (path: string) => void;
  onRescanTrack: (path: string) => void;
  onFixMatch: (path: string) => void;
}

export default function FoldersView({
  entries,
  dir,
  loading,
  selectedTrackPath,
  trackMenuPath,
  trackMenuPosition,
  canPlay,
  formatMs,
  onRescan,
  onNavigateUp,
  onBackToRoot,
  onSelectDir,
  onSelectTrack,
  onToggleMenu,
  onPlay,
  onQueue,
  onPlayNext,
  onRescanTrack,
  onFixMatch
}: FoldersViewProps) {
  return (
    <div className="card">
      <div className="card-header actions-only">
        <div className="card-actions">
          <span className="pill">{entries.length} items</span>
          <button className="btn ghost small" onClick={onRescan}>
            Rescan
          </button>
        </div>
      </div>
      <div className="library-path">
        <span className="muted small">Path</span>
        <span className="mono">{dir ?? "Loading..."}</span>
      </div>
      <div className="library-actions">
        <button className="btn ghost" disabled={!dir} onClick={onNavigateUp}>
          Up one level
        </button>
        <button className="btn ghost" onClick={onBackToRoot} disabled={!dir}>
          Back to root
        </button>
      </div>
      <LibraryList
        entries={entries}
        loading={loading}
        selectedTrackPath={selectedTrackPath}
        trackMenuPath={trackMenuPath}
        trackMenuPosition={trackMenuPosition}
        canPlay={canPlay}
        formatMs={formatMs}
        onSelectDir={onSelectDir}
        onSelectTrack={onSelectTrack}
        onToggleMenu={onToggleMenu}
        onPlay={onPlay}
        onQueue={onQueue}
        onPlayNext={onPlayNext}
        onRescan={onRescanTrack}
        onFixMatch={onFixMatch}
      />
    </div>
  );
}
