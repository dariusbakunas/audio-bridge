import { useCallback } from "react";
import { postJson } from "../api";
import { TrackSummary } from "../types";

interface QueueActionsOptions {
  sessionId: string | null;
  setError: (message: string | null) => void;
}

export function useQueueActions({ sessionId, setError }: QueueActionsOptions) {
  const requireSessionId = () => {
    if (!sessionId) {
      throw new Error("No active session. Reconnect and try again.");
    }
    return sessionId;
  };

  const handleNext = useCallback(async () => {
    try {
      const sid = requireSessionId();
      await postJson(`/sessions/${encodeURIComponent(sid)}/queue/next`);
    } catch (err) {
      setError((err as Error).message);
    }
  }, [sessionId, setError]);

  const handlePrevious = useCallback(async () => {
    try {
      const sid = requireSessionId();
      await postJson(`/sessions/${encodeURIComponent(sid)}/queue/previous`);
    } catch (err) {
      setError((err as Error).message);
    }
  }, [sessionId, setError]);

  const handleQueue = useCallback(
    async (trackId: number) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue`, { track_ids: [trackId] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handleQueueAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      if (!track.id) return;
      await handleQueue(track.id);
    },
    [handleQueue]
  );

  const handlePlayNext = useCallback(
    async (trackId: number) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue/next/add`, { track_ids: [trackId] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handleQueueRemove = useCallback(
    async (trackId: number) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue/remove`, { track_id: trackId });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handleQueueClear = useCallback(
    async (clearQueue: boolean, clearHistory: boolean) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue/clear`, {
          clear_queue: clearQueue,
          clear_history: clearHistory
        });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handleQueuePlayFrom = useCallback(
    async (trackId: number) => {
      try {
        const sid = requireSessionId();
        const endpoint = `/sessions/${encodeURIComponent(sid)}/queue/play_from`;
        await postJson(endpoint, { track_id: trackId });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
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
