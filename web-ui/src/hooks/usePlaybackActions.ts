import { useCallback } from "react";
import { postJson } from "../api";
import { TrackSummary } from "../types";

interface PlaybackActionsOptions {
  activeOutputId: string | null;
  rescanBusy: boolean;
  setError: (message: string | null) => void;
  setActiveOutputId: (id: string | null) => void;
  setRescanBusy: (busy: boolean) => void;
}

export function usePlaybackActions({
  activeOutputId,
  rescanBusy,
  setError,
  setActiveOutputId,
  setRescanBusy
}: PlaybackActionsOptions) {
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
      await postJson("/pause");
    } catch (err) {
      setError((err as Error).message);
    }
  }, [setError]);

  const handleNext = useCallback(async () => {
    try {
      await postJson("/queue/next");
    } catch (err) {
      setError((err as Error).message);
    }
  }, [setError]);

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
        await postJson("/outputs/select", { id });
        setActiveOutputId(id);
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setActiveOutputId, setError]
  );

  const handlePlay = useCallback(
    async (path: string) => {
      try {
        await postJson("/play", { path, queue_mode: "keep" });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [setError]
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
        await postJson("/play/album", {
          album_id: albumId,
          queue_mode: "replace",
          output_id: activeOutputId
        });
      } catch (err) {
        setError((err as Error).message);
      }
    },
    [activeOutputId, setError]
  );

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

  return {
    handleRescanLibrary,
    handleRescanTrack,
    handlePause,
    handleNext,
    handleRescan,
    handleSelectOutput,
    handlePlay,
    handlePlayAlbumTrack,
    handlePlayAlbumById,
    handleQueueAlbumTrack,
    handleQueue,
    handlePlayNext
  };
}
