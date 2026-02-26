import { useEffect, useRef, useState } from "react";
import { OutputInfo, SessionVolumeResponse, StatusResponse } from "../types";
import { Activity, List, Speaker, Volume2, VolumeX } from "lucide-react";
import PlayerControls from "./PlayerControls";

interface PlayerBarProps {
  status: StatusResponse | null;
  updatedAt: Date | null;
  nowPlayingCover: string | null;
  nowPlayingCoverFailed: boolean;
  placeholderCover: string;
  showSignalAction?: boolean;
  showSignalPath: boolean;
  canTogglePlayback: boolean;
  canGoPrevious: boolean;
  playButtonTitle?: string;
  queueHasItems: boolean;
  queueOpen: boolean;
  volume: SessionVolumeResponse | null;
  volumeBusy: boolean;
  showOutputAction?: boolean;
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
  onVolumeChange: (value: number) => void;
  onVolumeToggleMute: () => void;
  onSelectOutput: () => void;
}

export default function PlayerBar({
  status,
  updatedAt,
  nowPlayingCover,
  nowPlayingCoverFailed,
  placeholderCover,
  showSignalAction = true,
  showSignalPath,
  canTogglePlayback,
  canGoPrevious,
  playButtonTitle,
  queueHasItems,
  queueOpen,
  volume,
  volumeBusy,
  showOutputAction = true,
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
  onVolumeChange,
  onVolumeToggleMute,
  onSelectOutput
}: PlayerBarProps) {
  const compactQuery = "(max-width: 1760px)";
  const [compactVolume, setCompactVolume] = useState<boolean>(() =>
    typeof window !== "undefined" ? window.matchMedia(compactQuery).matches : false
  );
  const [clockMs, setClockMs] = useState<number>(() => Date.now());
  const [volumePopoverOpen, setVolumePopoverOpen] = useState<boolean>(false);
  const [volumeDragging, setVolumeDragging] = useState<boolean>(false);
  const volumeRootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const media = window.matchMedia(compactQuery);
    const update = () => setCompactVolume(media.matches);
    update();
    media.addEventListener("change", update);
    return () => {
      media.removeEventListener("change", update);
    };
  }, []);

  useEffect(() => {
    if (!compactVolume) {
      setVolumePopoverOpen(false);
    }
  }, [compactVolume]);

  useEffect(() => {
    if (!volumePopoverOpen) return;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (volumeRootRef.current && target && !volumeRootRef.current.contains(target)) {
        setVolumePopoverOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setVolumePopoverOpen(false);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [volumePopoverOpen]);

  useEffect(() => {
    if (!status?.now_playing_track_id || status?.paused) {
      return;
    }
    const timer = window.setInterval(() => {
      setClockMs(Date.now());
    }, 500);
    return () => {
      window.clearInterval(timer);
    };
  }, [status?.now_playing_track_id, status?.paused]);

  const showPlayIcon = !status?.now_playing_track_id || Boolean(status?.paused);
  const outputBitDepth =
    deriveOutputBitDepth(status?.output_sample_format) ?? status?.source_bit_depth;
  const outputRate = status?.output_sample_rate ?? status?.sample_rate;
  const sourceRate = status?.sample_rate;
  const sourceBitDepth = status?.source_bit_depth;
  const volumeAvailable = Boolean(volume?.available);
  const volumeValue = Math.max(0, Math.min(100, Math.round(volume?.value ?? 100)));
  const [volumeDraft, setVolumeDraft] = useState<number>(volumeValue);
  const volumeMuted = Boolean(volume?.muted);
  const VolumeIcon = volumeMuted ? VolumeX : Volume2;
  const volumeUnavailableHint = "Volume control is unavailable for the current output.";

  useEffect(() => {
    if (!volumeDragging) {
      setVolumeDraft(volumeValue);
    }
  }, [volumeDragging, volumeValue]);

  const commitVolume = () => {
    if (!volumeAvailable) return;
    setVolumeDragging(false);
    onVolumeChange(volumeDraft);
  };

  const displayedVolume = volumeDragging ? volumeDraft : volumeValue;
  const displayedElapsedMs = (() => {
    if (status?.elapsed_ms === null || status?.elapsed_ms === undefined) {
      return status?.elapsed_ms ?? null;
    }
    if (status?.paused || !updatedAt) {
      return status.elapsed_ms;
    }
    const base = status.elapsed_ms;
    const extra = Math.max(0, clockMs - updatedAt.getTime());
    const advanced = base + extra;
    return status.duration_ms ? Math.min(advanced, status.duration_ms) : advanced;
  })();
  return (
    <div className="player-bar">
      <div className="player-progress">
        <div className="player-progress-track" />
        <div
          className="player-progress-fill"
          style={{
            width:
              status?.duration_ms && displayedElapsedMs !== null && displayedElapsedMs !== undefined
                ? `${Math.min(100, (displayedElapsedMs / status.duration_ms) * 100)}%`
                : "0%"
          }}
        />
        <div
          className="player-progress-handle"
          style={{
            left:
              status?.duration_ms && displayedElapsedMs !== null && displayedElapsedMs !== undefined
                ? `${Math.min(100, (displayedElapsedMs / status.duration_ms) * 100)}%`
                : "0%"
          }}
        />
      </div>
      <div className="player-left">
        {status?.title || status?.now_playing_track_id ? (
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
            {status?.title ?? "Nothing playing"}
          </div>
          <div className="muted small">
            {status?.artist ?? (status?.now_playing_track_id ? "Unknown artist" : "Select a track to start")}
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
          elapsedLabel={formatMs(displayedElapsedMs)}
          durationLabel={formatMs(status?.duration_ms)}
          onPrimaryAction={onPrimaryAction}
          onPrevious={onPrevious}
          onNext={onNext}
        />
      </div>
      <div className="player-right">
        <div className="player-actions">
          {showSignalAction ? (
            <button
              className={`player-action player-action-signal${showSignalPath ? "" : " disabled"}`}
              onClick={onSignalOpen}
              disabled={!showSignalPath}
              aria-label="Signal details"
            >
              <Activity className="icon" aria-hidden="true" />
              <span className="player-action-label">
                {sourceRate && sourceBitDepth && outputRate && outputBitDepth
                  ? `SRC ${Math.round(sourceRate / 1000)}kHz/${sourceBitDepth} → OUT ${Math.round(
                      outputRate / 1000
                    )}kHz/${outputBitDepth}`
                  : outputRate && outputBitDepth
                    ? `${Math.round(outputRate / 1000)}kHz/${outputBitDepth}`
                    : "--/--"}
              </span>
            </button>
          ) : null}
          {showOutputAction ? (
            <button className="player-action player-action-output" onClick={onSelectOutput}>
              <Speaker className="icon" aria-hidden="true" />
              <span className="player-action-label">{activeOutput?.name ?? "Select output"}</span>
            </button>
          ) : null}
          {compactVolume ? (
            <div
              ref={volumeRootRef}
              className={`player-volume compact${volumeAvailable ? "" : " disabled"}`}
              title={volumeAvailable ? undefined : volumeUnavailableHint}
            >
              <button
                className="icon-btn volume-toggle-btn"
                aria-label="Volume"
                onClick={() => setVolumePopoverOpen((value) => !value)}
                disabled={!volumeAvailable}
                title={volumeAvailable ? `Volume ${displayedVolume}%` : volumeUnavailableHint}
              >
                <VolumeIcon className="icon" aria-hidden="true" />
              </button>
              {volumePopoverOpen ? (
                <div className="player-volume-popover" role="dialog" aria-label="Volume control">
                  <input
                    className="player-volume-slider-vertical"
                    type="range"
                    min={0}
                    max={100}
                    step={1}
                    value={displayedVolume}
                    onChange={(event) => setVolumeDraft(Number(event.target.value))}
                    onPointerDown={() => setVolumeDragging(true)}
                    onPointerUp={commitVolume}
                    onBlur={commitVolume}
                    disabled={!volumeAvailable}
                    aria-label="Volume"
                  />
                  <button
                    className="player-volume-popover-mute"
                    type="button"
                    onClick={onVolumeToggleMute}
                    disabled={!volumeAvailable || volumeBusy}
                  >
                    {volumeMuted ? "Unmute" : "Mute"}
                  </button>
                  <span className="player-volume-popover-value">{displayedVolume}%</span>
                </div>
              ) : null}
            </div>
          ) : (
            <div
              className={`player-volume${volumeAvailable ? "" : " disabled"}`}
              title={volumeAvailable ? undefined : volumeUnavailableHint}
            >
              <button
                className="icon-btn volume-toggle-btn"
                aria-label={volumeMuted ? "Unmute" : "Mute"}
                onClick={onVolumeToggleMute}
                disabled={!volumeAvailable || volumeBusy}
                title={volumeAvailable ? (volumeMuted ? "Unmute" : "Mute") : volumeUnavailableHint}
              >
                <VolumeIcon className="icon" aria-hidden="true" />
              </button>
              <input
                className="player-volume-slider"
                type="range"
                min={0}
                max={100}
                step={1}
                value={displayedVolume}
                onChange={(event) => setVolumeDraft(Number(event.target.value))}
                onPointerDown={() => setVolumeDragging(true)}
                onPointerUp={commitVolume}
                onBlur={commitVolume}
                disabled={!volumeAvailable}
                aria-label="Volume"
                title={volumeAvailable ? `Volume ${displayedVolume}%` : volumeUnavailableHint}
              />
              <span className="player-volume-value">
                {volumeAvailable ? `${displayedVolume}%` : "--"}
              </span>
            </div>
          )}
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

function deriveOutputBitDepth(format?: string | null): number | null {
  if (!format) {
    return null;
  }
  const upper = format.toUpperCase();
  if (upper.includes("I16")) return 16;
  if (upper.includes("I24")) return 24;
  if (upper.includes("I32")) return 32;
  if (upper.includes("F32")) return 32;
  if (upper.includes("F64")) return 64;
  return null;
}
