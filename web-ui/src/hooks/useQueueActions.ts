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
    async (path: string) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue`, { paths: [path] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
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
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue/next/add`, { paths: [path] });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handleQueueRemove = useCallback(
    async (path: string) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/queue/remove`, { path });
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
    async (payload: { trackId?: number; path?: string }) => {
      try {
        const sid = requireSessionId();
        const endpoint = `/sessions/${encodeURIComponent(sid)}/queue/play_from`;
        if (payload.trackId) {
          await postJson(endpoint, { track_id: payload.trackId });
        } else if (payload.path) {
          await postJson(endpoint, { path: payload.path });
        } else {
          throw new Error("Missing track id or path for queue playback.");
        }
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
