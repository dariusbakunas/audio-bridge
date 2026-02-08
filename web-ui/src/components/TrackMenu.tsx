import { CSSProperties } from "react";
import type { MouseEvent } from "react";

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
  onEditMetadata
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
        <span className="track-menu-dots" aria-hidden="true"></span>
      </button>
      {open ? (
        <div
          className="track-menu"
          onClick={(event) => event.stopPropagation()}
          data-track-menu="true"
          style={menuStyle}
        >
          <button className="track-menu-item" disabled={!canPlay} onClick={onPlay}>
            Play
          </button>
          <button className="track-menu-item" onClick={onQueue}>
            Queue
          </button>
          <button className="track-menu-item" disabled={!canPlay} onClick={onPlayNext}>
            Play next
          </button>
          {onFixMatch ? (
            <button className="track-menu-item" onClick={onFixMatch}>
              Fix MusicBrainz match
            </button>
          ) : null}
          {onEditMetadata ? (
            <button className="track-menu-item" onClick={onEditMetadata}>
              Edit file metadata
            </button>
          ) : null}
          <button className="track-menu-item" onClick={onRescan}>
            Rescan metadata
          </button>
        </div>
      ) : null}
    </div>
  );
}
