import { OutputInfo, StatusResponse } from "../types";
import { Activity, List, Volume2 } from "lucide-react";
import PlayerControls from "./PlayerControls";

interface PlayerBarProps {
  status: StatusResponse | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  placeholderCover: string;
  showSignalPath: boolean;
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  playButtonTitle?: string;
  queueHasItems: boolean;
  queueOpen: boolean;
  activeOutput: OutputInfo | null;
  activeAlbumId: number | null;
  uiBuildId: string;
  formatMs: (ms?: number | null) => string;
  onCoverError: () => void;
  onAlbumNavigate: (albumId: number) => void;
  onPrimaryAction: () => void;
  onPrevious: () => void;
  onNext: () => void;
  onSignalOpen: () => void;
  onQueueOpen: () => void;
  onSelectOutput: () => void;
}

export default function PlayerBar({
  status,
  nowPlayingCover,
  nowPlayingCoverFailed,
  placeholderCover,
  showSignalPath,
  canTogglePlayback,
  canGoPrevious,
  playButtonTitle,
  queueHasItems,
  queueOpen,
  activeOutput,
  activeAlbumId,
  uiBuildId,
  formatMs,
  onCoverError,
  onAlbumNavigate,
  onPrimaryAction,
  onPrevious,
  onNext,
  onSignalOpen,
  onQueueOpen,
  onSelectOutput
}: PlayerBarProps) {
  const showPlayIcon = !status?.now_playing || Boolean(status?.paused);
  return (
    <div className="player-bar">
      <div className="player-progress">
        <div className="player-progress-track" />
        <div
          className="player-progress-fill"
          style={{
            width:
              status?.duration_ms && status?.elapsed_ms
                ? `${Math.min(100, (status.elapsed_ms / status.duration_ms) * 100)}%`
                : "0%"
          }}
        />
        <div
          className="player-progress-handle"
          style={{
            left:
              status?.duration_ms && status?.elapsed_ms
                ? `${Math.min(100, (status.elapsed_ms / status.duration_ms) * 100)}%`
                : "0%"
          }}
        />
      </div>
      <div className="player-left">
        {status?.title || status?.now_playing ? (
          activeAlbumId ? (
            <button
              className="album-art album-art-button"
              type="button"
              onClick={() => onAlbumNavigate(activeAlbumId)}
              aria-label="Go to album"
            >
            {nowPlayingCover && !nowPlayingCoverFailed ? (
              <img
                className="album-art-image"
                src={nowPlayingCover}
                alt={status?.album ?? status?.title ?? "Album art"}
                onError={onCoverError}
              />
            ) : (
              <img
                className="album-art-image"
                src={placeholderCover}
                alt={status?.album ?? status?.title ?? "Album art"}
              />
            )}
          </button>
        ) : (
          <div className="album-art">
            {nowPlayingCover && !nowPlayingCoverFailed ? (
              <img
                className="album-art-image"
                src={nowPlayingCover}
                alt={status?.album ?? status?.title ?? "Album art"}
                onError={onCoverError}
              />
            ) : (
              <img
                className="album-art-image"
                src={placeholderCover}
                alt={status?.album ?? status?.title ?? "Album art"}
              />
            )}
          </div>
        )
        ) : null}
        <div>
          <div className="track-title">
            {status?.title ?? status?.now_playing ?? "Nothing playing"}
          </div>
          <div className="muted small">
            {status?.artist ?? (status?.now_playing ? "Unknown artist" : "Select a track to start")}
          </div>
        </div>
      </div>
      <div className="player-middle">
        <PlayerControls
          canTogglePlayback={canTogglePlayback}
          canGoPrevious={canGoPrevious}
          isPaused={showPlayIcon}
          playButtonTitle={playButtonTitle}
          queueHasItems={queueHasItems}
          elapsedLabel={formatMs(status?.elapsed_ms)}
          durationLabel={formatMs(status?.duration_ms)}
          onPrimaryAction={onPrimaryAction}
          onPrevious={onPrevious}
          onNext={onNext}
        />
      </div>
      <div className="player-right">
        <div className="player-actions">
          <button
            className={`player-action player-action-signal${showSignalPath ? "" : " disabled"}`}
            onClick={onSignalOpen}
            disabled={!showSignalPath}
            aria-label="Signal details"
          >
            <Activity className="icon" aria-hidden="true" />
            <span className="player-action-label">
              {status?.sample_rate && status?.source_bit_depth
                ? `${Math.round(status.sample_rate / 1000)}/${status.source_bit_depth}`
                : "--/--"}
            </span>
          </button>
          <button className="player-action player-action-output" onClick={onSelectOutput}>
            <Volume2 className="icon" aria-hidden="true" />
            <span className="player-action-label">{activeOutput?.name ?? "Select output"}</span>
          </button>
          <button
            className={`icon-btn queue-btn${queueOpen ? " active" : ""}`}
            aria-label="Queue"
            onClick={onQueueOpen}
          >
            <List className="icon" aria-hidden="true" />
          </button>
        </div>
        <div className="muted small build-footer">UI build: {uiBuildId}</div>
      </div>
    </div>
  );
}
