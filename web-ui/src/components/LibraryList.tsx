import { LibraryEntry } from "../types";
import TrackMenu from "./TrackMenu";

interface LibraryListProps {
  entries: LibraryEntry[];
  loading: boolean;
  selectedTrackPath: string | null;
  trackMenuPath: string | null;
  trackMenuPosition: { top: number; right: number } | null;
  canPlay: boolean;
  formatMs: (ms?: number | null) => string;
  onSelectDir: (path: string) => void;
  onSelectTrack: (path: string) => void;
  onToggleMenu: (path: string, target: HTMLElement) => void;
  onPlay: (path: string) => void;
  onQueue: (path: string) => void;
  onPlayNext: (path: string) => void;
  onRescan: (path: string) => void;
}

export default function LibraryList({
  entries,
  loading,
  selectedTrackPath,
  trackMenuPath,
  trackMenuPosition,
  canPlay,
  formatMs,
  onSelectDir,
  onSelectTrack,
  onToggleMenu,
  onPlay,
  onQueue,
  onPlayNext,
  onRescan
}: LibraryListProps) {
  return (
    <div className="library-list">
      {loading ? <p className="muted">Loading library...</p> : null}
      {!loading &&
        entries.map((entry) => {
          if (entry.kind === "dir") {
            return (
              <button
                key={entry.path}
                className="library-row"
                onClick={() => onSelectDir(entry.path)}
              >
                <div>
                  <div className="library-title">{entry.name}</div>
                  <div className="muted small">Folder</div>
                </div>
                <span className="chip">Open</span>
              </button>
            );
          }
          const isSelected = selectedTrackPath === entry.path;
          const menuOpen = trackMenuPath === entry.path;
          const menuStyle = menuOpen && trackMenuPosition
            ? { top: trackMenuPosition.top, right: trackMenuPosition.right }
            : undefined;
          return (
            <button
              key={entry.path}
              type="button"
              className={`library-row track${isSelected ? " selected" : ""}`}
              onClick={() => onSelectTrack(entry.path)}
            >
              <div>
                <div className="library-title">{entry.file_name}</div>
                <div className="muted small">
                  {entry.artist ?? "Unknown artist"}
                  {entry.album ? ` - ${entry.album}` : ""}
                </div>
              </div>
              <div className="library-actions-inline">
                <span className="muted small">{formatMs(entry.duration_ms)}</span>
                <TrackMenu
                  open={menuOpen}
                  canPlay={canPlay}
                  menuStyle={menuStyle}
                  onToggle={(event) => onToggleMenu(entry.path, event.currentTarget)}
                  onPlay={() => onPlay(entry.path)}
                  onQueue={() => onQueue(entry.path)}
                  onPlayNext={() => onPlayNext(entry.path)}
                  onRescan={() => onRescan(entry.path)}
                />
              </div>
            </button>
          );
        })}
      {!loading && entries.length === 0 ? (
        <p className="muted">No entries found in this folder.</p>
      ) : null}
    </div>
  );
}
