import { Pause, Play, SkipBack, SkipForward } from "lucide-react";

interface PlayerControlsProps {
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  isPaused: boolean;
  playButtonTitle?: string;
  queueHasItems: boolean;
  elapsedLabel: string;
  durationLabel: string;
  onPrimaryAction: () => void;
  onPrevious: () => void;
  onNext: () => void;
}

export default function PlayerControls({
  canTogglePlayback,
  canGoPrevious,
  isPaused,
  playButtonTitle,
  queueHasItems,
  elapsedLabel,
  durationLabel,
  onPrimaryAction,
  onPrevious,
  onNext
}: PlayerControlsProps) {
  return (
    <div className="player-controls">
      <button
        className="icon-btn"
        aria-label="Previous"
        onClick={onPrevious}
        disabled={!canGoPrevious}
      >
        <SkipBack className="icon" aria-hidden="true" />
      </button>
      <div className="player-controls-main">
        <button
          className="icon-btn primary"
          onClick={onPrimaryAction}
          aria-label="Play or pause"
          disabled={!canTogglePlayback}
          title={playButtonTitle}
        >
          {isPaused ? (
            <Play className="icon" aria-hidden="true" />
          ) : (
            <Pause className="icon" aria-hidden="true" />
          )}
        </button>
        <div className="controls-time">
          <span>{elapsedLabel}</span>
          <span className="controls-time-sep">|</span>
          <span>{durationLabel}</span>
        </div>
      </div>
      <button
        className="icon-btn"
        onClick={onNext}
        aria-label="Next"
        disabled={!queueHasItems || !canTogglePlayback}
      >
        <SkipForward className="icon" aria-hidden="true" />
      </button>
    </div>
  );
}
