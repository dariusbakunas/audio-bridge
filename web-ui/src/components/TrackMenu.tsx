import { CSSProperties } from "react";
import type { MouseEvent } from "react";
import { Activity, ChevronRight, List, MoreVertical, Pencil, Play, RefreshCw, Search } from "lucide-react";

interface TrackMenuProps {
  open: boolean;
  canPlay: boolean;
  menuStyle?: CSSProperties;
  onToggle: (event: MouseEvent<HTMLButtonElement>) => void;
  onPlay: () => void;
  onQueue: () => void;
  onPlayNext: () => void;
  onRescan: () => void;
  onFixMatch?: () => void;
  onEditMetadata?: () => void;
  onAnalyze?: () => void;
}

export default function TrackMenu({
  open,
  canPlay,
  menuStyle,
  onToggle,
  onPlay,
  onQueue,
  onPlayNext,
  onRescan,
  onFixMatch,
  onEditMetadata,
  onAnalyze
}: TrackMenuProps) {
  return (
    <div className="track-menu-wrap" data-track-menu="true">
      <button
        className="track-menu-button"
        aria-label="Track options"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          onToggle(event);
        }}
        data-track-menu="true"
      >
        <MoreVertical className="icon" aria-hidden="true" />
      </button>
      {open ? (
        <div
          className="track-menu"
          onClick={(event) => event.stopPropagation()}
          data-track-menu="true"
          style={menuStyle}
        >
          <button className="track-menu-item" disabled={!canPlay} onClick={onPlay}>
            <Play className="track-menu-icon" aria-hidden="true" />
            Play
          </button>
          <button className="track-menu-item" onClick={onQueue}>
            <List className="track-menu-icon" aria-hidden="true" />
            Queue
          </button>
          <button className="track-menu-item" disabled={!canPlay} onClick={onPlayNext}>
            <ChevronRight className="track-menu-icon" aria-hidden="true" />
            Play next
          </button>
          {onFixMatch ? (
            <button className="track-menu-item" onClick={onFixMatch}>
              <Search className="track-menu-icon" aria-hidden="true" />
              Fix MusicBrainz match
            </button>
          ) : null}
          {onEditMetadata ? (
            <button className="track-menu-item" onClick={onEditMetadata}>
              <Pencil className="track-menu-icon" aria-hidden="true" />
              Edit file metadata
            </button>
          ) : null}
          {onAnalyze ? (
            <button className="track-menu-item" onClick={onAnalyze}>
              <Activity className="track-menu-icon" aria-hidden="true" />
              Analyze track
            </button>
          ) : null}
          <button className="track-menu-item" onClick={onRescan}>
            <RefreshCw className="track-menu-icon" aria-hidden="true" />
            Rescan metadata
          </button>
        </div>
      ) : null}
    </div>
  );
}
