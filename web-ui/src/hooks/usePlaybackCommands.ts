import { useCallback } from "react";

import { fetchJson, postJson } from "../api";
import { TrackListResponse, TrackSummary } from "../types";
import { LocalPlaybackCommand } from "./useLocalPlayback";

type UsePlaybackCommandsArgs = {
  isLocalSession: boolean;
  sessionId: string | null;
  reportError: (message: string) => void;
  applyLocalPlayback: (payload: LocalPlaybackCommand | null) => Promise<void>;
  requestLocalCommand: (
    endpoint: string,
    body?: Record<string, string | number | boolean | null | undefined>
  ) => Promise<LocalPlaybackCommand | null>;
  toggleLocalPause: () => Promise<boolean>;
  handlePauseRemote: () => Promise<void>;
  handlePlayRemote: (trackId: number) => Promise<void>;
  handlePlayAlbumTrackRemote: (track: TrackSummary) => Promise<void>;
  handlePlayAlbumByIdRemote: (albumId: number) => Promise<void>;
  handleNextRemote: () => Promise<void>;
  handlePreviousRemote: () => Promise<void>;
  handleQueuePlayFromRemote: (trackId: number) => Promise<void>;
};

export function usePlaybackCommands({
  isLocalSession,
  sessionId,
  reportError,
  applyLocalPlayback,
  requestLocalCommand,
  toggleLocalPause,
  handlePauseRemote,
  handlePlayRemote,
  handlePlayAlbumTrackRemote,
  handlePlayAlbumByIdRemote,
  handleNextRemote,
  handlePreviousRemote,
  handleQueuePlayFromRemote
}: UsePlaybackCommandsArgs) {
  const handlePause = useCallback(async () => {
    if (isLocalSession) {
      await toggleLocalPause();
      return;
    }
    await handlePauseRemote();
  }, [handlePauseRemote, isLocalSession, toggleLocalPause]);

  const handlePlay = useCallback(
    async (trackId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayRemote(trackId);
          return;
        }
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, {
          track_ids: [trackId]
        });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [applyLocalPlayback, handlePlayRemote, isLocalSession, reportError, requestLocalCommand, sessionId]
  );

  const handleNext = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handleNextRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/next");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [applyLocalPlayback, handleNextRemote, isLocalSession, reportError, requestLocalCommand]);

  const handlePrevious = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handlePreviousRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/previous");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [applyLocalPlayback, handlePreviousRemote, isLocalSession, reportError, requestLocalCommand]);

  const handleQueuePlayFrom = useCallback(
    async (trackId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handleQueuePlayFromRemote(trackId);
          return;
        }
        const command = await requestLocalCommand("/queue/play_from", { track_id: trackId });
        await applyLocalPlayback(command);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      applyLocalPlayback,
      handleQueuePlayFromRemote,
      isLocalSession,
      reportError,
      requestLocalCommand,
      sessionId
    ]
  );

  const handlePlayAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumTrackRemote(track);
          return;
        }
        if (!track.id) {
          return;
        }
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, {
          track_ids: [track.id]
        });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      applyLocalPlayback,
      handlePlayAlbumTrackRemote,
      isLocalSession,
      reportError,
      requestLocalCommand,
      sessionId
    ]
  );

  const handlePlayAlbumById = useCallback(
    async (albumId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumByIdRemote(albumId);
          return;
        }
        const tracks = await fetchJson<TrackListResponse>(`/tracks?album_id=${albumId}&limit=500`);
        const trackIds = (tracks.items ?? [])
          .map((track) => track.id)
          .filter((id): id is number => Number.isFinite(id));
        if (!trackIds.length) {
          throw new Error("Album has no playable tracks.");
        }
        const base = `/sessions/${encodeURIComponent(sessionId)}/queue`;
        await postJson(`${base}/clear`, {
          clear_queue: true,
          clear_history: false
        });
        await postJson(base, { track_ids: trackIds });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      applyLocalPlayback,
      handlePlayAlbumByIdRemote,
      isLocalSession,
      reportError,
      requestLocalCommand,
      sessionId
    ]
  );

  return {
    handlePause,
    handlePlay,
    handleNext,
    handlePrevious,
    handleQueuePlayFrom,
    handlePlayAlbumTrack,
    handlePlayAlbumById
  };
}
