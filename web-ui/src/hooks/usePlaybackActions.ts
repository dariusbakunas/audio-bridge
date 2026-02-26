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
    async (trackId: number) => {
      if (rescanBusy) return;
      setRescanBusy(true);
      try {
        await postJson("/library/rescan/track", { track_id: trackId });
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
    async (id: string, force = false) => {
      try {
        const sid = requireSessionId();
        await postJson(`/sessions/${encodeURIComponent(sid)}/select-output`, {
          output_id: id,
          force
        });
        setActiveOutputId(id);
      } catch (err) {
        const message = (err as Error).message;
        setError(parseConflictMessage(message) ?? message);
      }
    },
    [sessionId, setActiveOutputId, setError]
  );

  const handlePlay = useCallback(
    async (trackId: number) => {
      try {
        const sid = requireSessionId();
        const base = `/sessions/${encodeURIComponent(sid)}/queue`;
        await postJson(`${base}/next/add`, { track_ids: [trackId] });
        await postJson(`${base}/next`);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handlePlayAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      try {
        const sid = requireSessionId();
        const base = `/sessions/${encodeURIComponent(sid)}/queue`;
        if (!track.id) return;
        await postJson(`${base}/next/add`, { track_ids: [track.id] });
        await postJson(`${base}/next`);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [sessionId, setError]
  );

  const handlePlayAlbumById = useCallback(
    async (albumId: number) => {
      if (!activeOutputId) return;
      try {
        const sid = requireSessionId();
        const tracks = await fetchJson<TrackListResponse>(
          `/tracks?album_id=${albumId}&limit=500`
        );
        const trackIds = (tracks.items ?? [])
          .map((track) => track.id)
          .filter((id): id is number => Number.isFinite(id));
        if (!trackIds.length) {
          throw new Error("Album has no playable tracks.");
        }
        const base = `/sessions/${encodeURIComponent(sid)}/queue`;
        await postJson(`${base}/clear`, {
          clear_queue: true,
          clear_history: false
        });
        await postJson(base, { track_ids: trackIds });
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
  const parseConflictMessage = (message: string): string | null => {
    try {
      const parsed = JSON.parse(message) as {
        error?: string;
        output_id?: string;
        held_by_session_id?: string;
      };
      if (parsed?.error === "output_in_use" && parsed.output_id && parsed.held_by_session_id) {
        return `Output is already in use (${parsed.output_id}) by session ${parsed.held_by_session_id}. Use Force to take it.`;
      }
    } catch {
      // ignore parse failures
    }
    if (message.includes("bridge_in_use")) {
      const bridgeId = /bridge_id=([^\s]+)/.exec(message)?.[1];
      const holder = /held_by_session_id=([^\s]+)/.exec(message)?.[1];
      if (bridgeId && holder) {
        return `Bridge ${bridgeId} is already in use by session ${holder}.`;
      }
    }
    return null;
  };
