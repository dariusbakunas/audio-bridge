import { Pause, Play, SkipBack, SkipForward } from "lucide-react";

import { StatusResponse } from "../types";

type NowPlayingScreenProps = {
  status: StatusResponse | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  placeholderCover: string;
  formatMs: (ms?: number | null) => string;
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  hasNowPlaying: boolean;
  isPaused: boolean;
  queueHasItems: boolean;
  onCoverError: () => void;
  onPrimaryAction: () => void | Promise<void>;
  onPrevious: () => void | Promise<void>;
  onNext: () => void | Promise<void>;
};

export default function NowPlayingScreen({
  status,
  nowPlayingCover,
  nowPlayingCoverFailed,
  placeholderCover,
  formatMs,
  canTogglePlayback,
  canGoPrevious,
  hasNowPlaying,
  isPaused,
  queueHasItems,
  onCoverError,
  onPrimaryAction,
  onPrevious,
  onNext
}: NowPlayingScreenProps) {
  const elapsedMs = status?.elapsed_ms ?? null;
  const durationMs = status?.duration_ms ?? null;
  const progressPercent =
    elapsedMs !== null && durationMs && durationMs > 0 ? Math.min(100, (elapsedMs / durationMs) * 100) : 0;

  return (
    <section className="now-playing-screen">
      <div className="now-playing-artwork">
        <img
          className="now-playing-image"
          src={nowPlayingCover && !nowPlayingCoverFailed ? nowPlayingCover : placeholderCover}
          alt={status?.album ?? status?.title ?? "Album art"}
          onError={onCoverError}
        />
      </div>

      <div className="now-playing-copy">
        <div className="now-playing-title">{status?.title ?? "Nothing playing"}</div>
        <div className="now-playing-artist">{status?.artist ?? "Unknown artist"}</div>
        <div className="now-playing-album">{status?.album ?? ""}</div>
      </div>

      <div className="now-playing-progress">
        <div className="now-playing-progress-track">
          <div className="now-playing-progress-fill" style={{ width: `${progressPercent}%` }} />
        </div>
        <div className="now-playing-times">
          <span>{formatMs(elapsedMs)}</span>
          <span>{formatMs(durationMs)}</span>
        </div>
      </div>

      <div className="now-playing-controls">
        <button
          className="icon-btn"
          aria-label="Previous"
          onClick={() => {
            void onPrevious();
          }}
          disabled={!canGoPrevious}
          type="button"
        >
          <SkipBack className="icon" aria-hidden="true" />
        </button>
        <button
          className="icon-btn primary now-playing-primary"
          onClick={() => {
            void onPrimaryAction();
          }}
          aria-label="Play or pause"
          disabled={!canTogglePlayback}
          type="button"
        >
          {!canTogglePlayback || !hasNowPlaying || isPaused ? (
            <Play className="icon" aria-hidden="true" />
          ) : (
            <Pause className="icon" aria-hidden="true" />
          )}
        </button>
        <button
          className="icon-btn"
          onClick={() => {
            void onNext();
          }}
          aria-label="Next"
          disabled={!queueHasItems || !canTogglePlayback}
          type="button"
        >
          <SkipForward className="icon" aria-hidden="true" />
        </button>
      </div>
    </section>
  );
}
