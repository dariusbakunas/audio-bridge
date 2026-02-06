interface PlayerControlsProps {
  isPlaying: boolean;
  canTogglePlayback: boolean;
  showPlayIcon: boolean;
  playButtonTitle?: string;
  queueHasItems: boolean;
  onPrimaryAction: () => void;
  onNext: () => void;
  onSignalOpen: () => void;
  onQueueOpen: () => void;
}

export default function PlayerControls({
  isPlaying,
  canTogglePlayback,
  showPlayIcon,
  playButtonTitle,
  queueHasItems,
  onPrimaryAction,
  onNext,
  onSignalOpen,
  onQueueOpen
}: PlayerControlsProps) {
  return (
    <div className="player-controls">
      <button
        className={`icon-btn signal-btn${isPlaying ? " active" : ""}`}
        aria-label="Signal details"
        onClick={onSignalOpen}
        disabled={!isPlaying}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="3" y="10" width="2" height="4" rx="1" />
          <rect x="7" y="7" width="2" height="10" rx="1" />
          <rect x="11" y="4" width="2" height="16" rx="1" />
          <rect x="15" y="7" width="2" height="10" rx="1" />
          <rect x="19" y="10" width="2" height="4" rx="1" />
        </svg>
      </button>
      <button className="icon-btn" aria-label="Previous" disabled>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="3" y="5" width="2" height="14" rx="1" />
          <polygon points="21,5 13,12 21,19" />
          <polygon points="13,5 5,12 13,19" />
        </svg>
      </button>
      <button
        className="icon-btn primary"
        onClick={onPrimaryAction}
        aria-label="Play or pause"
        disabled={!canTogglePlayback}
        title={playButtonTitle}
      >
        {showPlayIcon ? (
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <polygon points="7,5 19,12 7,19" />
          </svg>
        ) : (
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="6" y="5" width="4" height="14" rx="1" />
            <rect x="14" y="5" width="4" height="14" rx="1" />
          </svg>
        )}
      </button>
      <button
        className="icon-btn"
        onClick={onNext}
        aria-label="Next"
        disabled={!queueHasItems}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="19" y="5" width="2" height="14" rx="1" />
          <polygon points="3,5 11,12 3,19" />
          <polygon points="11,5 19,12 11,19" />
        </svg>
      </button>
      <button className="icon-btn queue-btn" aria-label="Queue" onClick={onQueueOpen}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="4" y="6" width="16" height="2" rx="1" />
          <rect x="4" y="11" width="16" height="2" rx="1" />
          <rect x="4" y="16" width="10" height="2" rx="1" />
        </svg>
      </button>
    </div>
  );
}
