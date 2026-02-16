import { useCallback } from "react";
import { postJson } from "../api";
import { TrackSummary } from "../types";

interface QueueActionsOptions {
  setError: (message: string | null) => void;
}

export function useQueueActions({ setError }: QueueActionsOptions) {
  const handleNext = useCallback(async () => {
    try {
      await postJson("/queue/next");
    } catch (err) {
      setError((err as Error).message);
    }
  }, [setError]);

  const handlePrevious = useCallback(async () => {
    try {
      await postJson("/queue/previous");
    } catch (err) {
      setError((err as Error).message);
    }
  }, [setError]);

  const handleQueue = useCallback(
    async (path: string) => {
      try {
        await postJson("/queue", { paths: [path] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
  );

  const handleQueueAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      if (!track.path) return;
      await handleQueue(track.path);
    },
    [handleQueue]
  );

  const handlePlayNext = useCallback(
    async (path: string) => {
      try {
        await postJson("/queue/next/add", { paths: [path] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
  );

  const handleQueueRemove = useCallback(
    async (path: string) => {
      try {
        await postJson("/queue/remove", { path });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
  );

  const handleQueueClear = useCallback(
    async (clearQueue: boolean, clearHistory: boolean) => {
      try {
        await postJson("/queue/clear", {
          clear_queue: clearQueue,
          clear_history: clearHistory
        });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
  );

  const handleQueuePlayFrom = useCallback(
    async (payload: { trackId?: number; path?: string }) => {
      try {
        if (payload.trackId) {
          await postJson("/queue/play_from", { track_id: payload.trackId });
        } else if (payload.path) {
          await postJson("/queue/play_from", { path: payload.path });
        } else {
          throw new Error("Missing track id or path for queue playback.");
        }
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
  );

  return {
    handleNext,
    handlePrevious,
    handleQueue,
    handleQueueAlbumTrack,
    handlePlayNext,
    handleQueueRemove,
    handleQueueClear,
    handleQueuePlayFrom
  };
}
