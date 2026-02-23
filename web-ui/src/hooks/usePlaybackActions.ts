import { useCallback } from "react";
import { fetchJson, postJson } from "../api";
import { TrackListResponse, TrackSummary } from "../types";

interface PlaybackActionsOptions {
  sessionId: string | null;
  activeOutputId: string | null;
  rescanBusy: boolean;
  setError: (message: string | null) => void;
  setActiveOutputId: (id: string | null) => void;
  setRescanBusy: (busy: boolean) => void;
}

export function usePlaybackActions({
  sessionId,
  activeOutputId,
  rescanBusy,
  setError,
  setActiveOutputId,
  setRescanBusy
}: PlaybackActionsOptions) {
  const requireSessionId = () => {
    if (!sessionId) {
      throw new Error("No active session. Reconnect and try again.");
    }
    return sessionId;
  };

  const handleRescanLibrary = useCallback(async () => {
    if (rescanBusy) return;
    setRescanBusy(true);
    try {
      await postJson("/library/rescan");
      setError(null);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setRescanBusy(false);
    }
  }, [rescanBusy, setRescanBusy, setError]);

  const handleRescanTrack = useCallback(
    async (path: string) => {
      if (rescanBusy) return;
      setRescanBusy(true);
      try {
        await postJson("/library/rescan/track", { path });
        setError(null);
      } catch (err) {
        setError((err as Error).message);
      } finally {
        setRescanBusy(false);
      }
    },
    [rescanBusy, setRescanBusy, setError]
  );

  const handlePause = useCallback(async () => {
    try {
      const sid = requireSessionId();
      await postJson(`/sessions/${encodeURIComponent(sid)}/pause`);
    } catch (err) {
      setError((err as Error).message);
    }
  }, [sessionId, setError]);

  const handleRescan = useCallback(async () => {
    try {
      await postJson("/library/rescan");
    } catch (err) {
      setError((err as Error).message);
    }
  }, [setError]);

  const handleSelectOutput = useCallback(
    async (id: string) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/select-output`, { output_id: id });
        setActiveOutputId(id);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setActiveOutputId, setError]
  );

  const handlePlay = useCallback(
    async (path: string) => {
      try {
        const sid = requireSessionId();
        const base = `/sessions/${encodeURIComponent(sid)}/queue`;
        await postJson(`${base}/next/add`, { paths: [path] });
        await postJson(`${base}/next`);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handlePlayAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      if (!track.path) return;
      await handlePlay(track.path);
    },
    [handlePlay]
  );

  const handlePlayAlbumById = useCallback(
    async (albumId: number) => {
      if (!activeOutputId) return;
      try {
        const sid = requireSessionId();
        const tracks = await fetchJson<TrackListResponse>(
          `/tracks?album_id=${albumId}&limit=500`
        );
        const paths = (tracks.items ?? [])
          .map((track) => track.path)
          .filter((path): path is string => Boolean(path));
        if (!paths.length) {
          throw new Error("Album has no playable tracks.");
        }
        const base = `/sessions/${encodeURIComponent(sid)}/queue`;
        await postJson(`${base}/clear`, {
          clear_queue: true,
          clear_history: false
        });
        await postJson(base, { paths });
        await postJson(`${base}/next`);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, activeOutputId, setError]
  );

  return {
    handleRescanLibrary,
    handleRescanTrack,
    handlePause,
    handleRescan,
    handleSelectOutput,
    handlePlay,
    handlePlayAlbumTrack,
    handlePlayAlbumById
  };
}
