import { OutputInfo, StatusResponse } from "../types";
import PlayerControls from "./PlayerControls";

interface PlayerBarProps {
  status: StatusResponse | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  placeholderCover: string;
  isPlaying: boolean;
  canTogglePlayback: boolean;
  showPlayIcon: boolean;
  playButtonTitle?: string;
  queueHasItems: boolean;
  activeOutput: OutputInfo | null;
  activeAlbumId: number | null;
  uiBuildId: string;
  formatMs: (ms?: number | null) => string;
  onCoverError: () => void;
  onAlbumNavigate: (albumId: number) => void;
  onPrimaryAction: () => void;
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
  isPlaying,
  canTogglePlayback,
  showPlayIcon,
  playButtonTitle,
  queueHasItems,
  activeOutput,
  activeAlbumId,
  uiBuildId,
  formatMs,
  onCoverError,
  onAlbumNavigate,
  onPrimaryAction,
  onNext,
  onSignalOpen,
  onQueueOpen,
  onSelectOutput
}: PlayerBarProps) {
  return (
    <div className="player-bar">
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
          isPlaying={isPlaying}
          canTogglePlayback={canTogglePlayback}
          showPlayIcon={showPlayIcon}
          playButtonTitle={playButtonTitle}
          queueHasItems={queueHasItems}
          onPrimaryAction={onPrimaryAction}
          onNext={onNext}
          onSignalOpen={onSignalOpen}
          onQueueOpen={onQueueOpen}
        />
        <div className="progress">
          <div className="progress-track"></div>
          <div
            className="progress-fill"
            style={{
              width:
                status?.duration_ms && status?.elapsed_ms
                  ? `${Math.min(100, (status.elapsed_ms / status.duration_ms) * 100)}%`
                  : "0%"
            }}
          ></div>
        </div>
        <div className="meta-row">
          <span>
            {formatMs(status?.elapsed_ms)} / {formatMs(status?.duration_ms)}
          </span>
          <span>{status?.format ?? "—"}</span>
        </div>
      </div>
      <div className="player-right">
        <div className="output-chip">
          <span className="muted small">Output</span>
          <span>{activeOutput?.name ?? "No output"}</span>
        </div>
        <button className="btn ghost small" onClick={onSelectOutput}>
          Select output
        </button>
        <div className="muted small build-footer">UI build: {uiBuildId}</div>
      </div>
    </div>
  );
}
