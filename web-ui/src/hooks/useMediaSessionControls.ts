import { useCallback, useEffect } from "react";

import { QueueItem, StatusResponse } from "../types";

type UseMediaSessionControlsArgs = {
  status: StatusResponse | null;
  nowPlayingCover: string | null;
  hasNowPlaying: boolean;
  replayTrackId: number | null;
  isLocalSession: boolean;
  sessionId: string | null;
  queue: QueueItem[];
  effectiveNowPlayingTrackId: number | null;
  resumeLocalFromStatus: (trackId: number | null, elapsedMs: number | null) => Promise<boolean>;
  handlePause: () => Promise<void>;
  handleQueuePlayFrom: (trackId: number) => Promise<void>;
  handlePrevious: () => Promise<void>;
  handleNext: () => Promise<void>;
  reportError: (message: string) => void;
};

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

export function useMediaSessionControls({
  status,
  nowPlayingCover,
  hasNowPlaying,
  replayTrackId,
  isLocalSession,
  sessionId,
  queue,
  effectiveNowPlayingTrackId,
  resumeLocalFromStatus,
  handlePause,
  handleQueuePlayFrom,
  handlePrevious,
  handleNext,
  reportError
}: UseMediaSessionControlsArgs) {
  const handlePrimaryAction = useCallback(async () => {
    if (hasNowPlaying) {
      if (isLocalSession && status?.paused && sessionId) {
        try {
          const currentQueueTrack = queue.find(
            (item) => item.kind === "track" && item.now_playing && item.id
          ) as { id?: number } | undefined;
          const replayId = currentQueueTrack?.id ?? effectiveNowPlayingTrackId;
          const resumed = await resumeLocalFromStatus(replayId ?? null, status.elapsed_ms ?? null);
          if (resumed) {
            return;
          }
        } catch (err) {
          reportError((err as Error).message);
          return;
        }
      }
      await handlePause();
      return;
    }
    if (replayTrackId !== null) {
      await handleQueuePlayFrom(replayTrackId);
    }
  }, [
    effectiveNowPlayingTrackId,
    handlePause,
    handleQueuePlayFrom,
    hasNowPlaying,
    isLocalSession,
    queue,
    replayTrackId,
    reportError,
    resumeLocalFromStatus,
    sessionId,
    status?.elapsed_ms,
    status?.paused
  ]);

  const handlePlayMedia = useCallback(async () => {
    if (hasNowPlaying) {
      if (status?.paused) {
        await handlePause();
      }
      return;
    }
    if (replayTrackId !== null) {
      await handleQueuePlayFrom(replayTrackId);
    }
  }, [handlePause, handleQueuePlayFrom, hasNowPlaying, replayTrackId, status?.paused]);

  const handlePauseMedia = useCallback(async () => {
    if (hasNowPlaying && !status?.paused) {
      await handlePause();
    }
  }, [handlePause, hasNowPlaying, status?.paused]);

  useEffect(() => {
    function handleKey(event: KeyboardEvent) {
      if (event.code !== "Space") return;
      if (event.repeat) return;
      if (isEditableTarget(event.target)) return;
      event.preventDefault();
      void handlePrimaryAction();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [handlePrimaryAction]);

  useEffect(() => {
    const mediaSession = navigator.mediaSession;
    if (!mediaSession) return;

    if (status?.title || status?.artist || status?.album) {
      const artwork = nowPlayingCover ? [{ src: nowPlayingCover, sizes: "512x512" }] : [];
      mediaSession.metadata = new MediaMetadata({
        title: status?.title ?? "",
        artist: status?.artist ?? "",
        album: status?.album ?? "",
        artwork
      });
    } else {
      mediaSession.metadata = null;
    }

    try {
      mediaSession.setActionHandler("play", () => {
        void handlePlayMedia();
      });
      mediaSession.setActionHandler("pause", () => {
        void handlePauseMedia();
      });
      mediaSession.setActionHandler("previoustrack", () => {
        void handlePrevious();
      });
      mediaSession.setActionHandler("nexttrack", () => {
        void handleNext();
      });
    } catch {
      // MediaSession action handlers are best-effort.
    }

    return () => {
      try {
        mediaSession.setActionHandler("play", null);
        mediaSession.setActionHandler("pause", null);
        mediaSession.setActionHandler("previoustrack", null);
        mediaSession.setActionHandler("nexttrack", null);
      } catch {
        // Best-effort cleanup.
      }
    };
  }, [
    handleNext,
    handlePauseMedia,
    handlePlayMedia,
    handlePrevious,
    nowPlayingCover,
    status?.album,
    status?.artist,
    status?.title
  ]);

  return {
    handlePrimaryAction
  };
}
