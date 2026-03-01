import { SetStateAction, useCallback, useEffect, useRef } from "react";

import { postJson, safeMediaUrl } from "../api";
import { QueueItem, StatusResponse } from "../types";

const LOCAL_PLAYBACK_SNAPSHOT_KEY_PREFIX = "audioHub.localPlaybackSnapshot:";

type LocalPlaybackSnapshot = {
  track_id: number;
  paused: boolean;
  elapsed_ms: number | null;
  duration_ms: number | null;
  title: string | null;
  artist: string | null;
  album: string | null;
  saved_at_ms: number;
};

export type LocalPlaybackCommand = {
  url: string;
  track_id: number;
};

function localPlaybackSnapshotKey(sessionId: string): string {
  return `${LOCAL_PLAYBACK_SNAPSHOT_KEY_PREFIX}${sessionId}`;
}

function loadLocalPlaybackSnapshot(sessionId: string): LocalPlaybackSnapshot | null {
  try {
    const raw = localStorage.getItem(localPlaybackSnapshotKey(sessionId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as LocalPlaybackSnapshot;
    if (!parsed?.track_id) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveLocalPlaybackSnapshot(sessionId: string, snapshot: LocalPlaybackSnapshot): void {
  try {
    localStorage.setItem(localPlaybackSnapshotKey(sessionId), JSON.stringify(snapshot));
  } catch {
    // ignore storage failures
  }
}

type UseLocalPlaybackArgs = {
  isLocalSession: boolean;
  sessionId: string | null;
  activeOutputId: string | null;
  queue: QueueItem[];
  status: StatusResponse | null;
  setStatus: (value: SetStateAction<StatusResponse | null>) => void;
  markUpdatedAt: () => void;
  reportError: (message: string) => void;
};

export function useLocalPlayback({
  isLocalSession,
  sessionId,
  activeOutputId,
  queue,
  status,
  setStatus,
  markUpdatedAt,
  reportError
}: UseLocalPlaybackArgs) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const localTrackIdRef = useRef<number | null>(null);

  const updateLocalStatusFromAudio = useCallback(
    (base?: Partial<StatusResponse>) => {
      if (!isLocalSession) return;
      const audio = audioRef.current;
      if (!audio) return;
      const hasTrack = Boolean(localTrackIdRef.current);
      setStatus((prev) => {
        const next: StatusResponse = {
          ...(prev ?? {}),
          ...base,
          now_playing_track_id: hasTrack ? localTrackIdRef.current : null,
          paused: hasTrack ? audio.paused : true,
          elapsed_ms:
            hasTrack && Number.isFinite(audio.currentTime)
              ? Math.floor(audio.currentTime * 1000)
              : null,
          duration_ms:
            hasTrack && Number.isFinite(audio.duration) ? Math.floor(audio.duration * 1000) : null
        };
        if (!hasTrack) {
          next.title = null;
          next.artist = null;
          next.album = null;
          next.output_sample_rate = null;
          next.channels = null;
        }
        return next;
      });
      markUpdatedAt();
    },
    [isLocalSession, markUpdatedAt, setStatus]
  );

  const applyLocalPlayback = useCallback(
    async (payload: LocalPlaybackCommand | null) => {
      const audio = audioRef.current;
      if (!audio) return;
      if (!payload?.url || !payload.track_id) {
        audio.pause();
        audio.removeAttribute("src");
        audio.load();
        localTrackIdRef.current = null;
        updateLocalStatusFromAudio();
        return;
      }
      const safeUrl = safeMediaUrl(payload.url);
      if (!safeUrl) {
        reportError("Rejected local playback URL.");
        return;
      }
      const queueTrack = queue.find((item) => item.kind === "track" && item.id === payload.track_id);
      localTrackIdRef.current = payload.track_id;
      audio.src = safeUrl;
      audio.load();
      await audio.play().catch(() => {});
      updateLocalStatusFromAudio({
        title: queueTrack?.kind === "track" ? (queueTrack.title ?? queueTrack.file_name) : null,
        artist: queueTrack?.kind === "track" ? (queueTrack.artist ?? null) : null,
        album: queueTrack?.kind === "track" ? (queueTrack.album ?? null) : null
      });
    },
    [queue, reportError, updateLocalStatusFromAudio]
  );

  const requestLocalCommand = useCallback(
    async (
      endpoint: string,
      body?: Record<string, string | number | boolean | null | undefined>
    ): Promise<LocalPlaybackCommand | null> => {
      if (!sessionId) return null;
      const response = await postJson<LocalPlaybackCommand | null>(
        `/sessions/${encodeURIComponent(sessionId)}${endpoint}`,
        body as any
      );
      if (!response || !response.url || !response.track_id) {
        return null;
      }
      return response;
    },
    [sessionId]
  );

  const toggleLocalPause = useCallback(async (): Promise<boolean> => {
    if (!isLocalSession) return false;
    const audio = audioRef.current;
    if (!audio || !localTrackIdRef.current) return true;
    if (audio.paused) {
      await audio.play().catch(() => {});
    } else {
      audio.pause();
    }
    updateLocalStatusFromAudio();
    return true;
  }, [isLocalSession, updateLocalStatusFromAudio]);

  const resumeLocalFromStatus = useCallback(
    async (replayTrackId: number | null, seekMs: number | null): Promise<boolean> => {
      if (!isLocalSession || !sessionId) return false;
      const audio = audioRef.current;
      const hasSource = Boolean(audio?.src);
      if (hasSource) return false;
      if (!replayTrackId) {
        throw new Error("Track ID is required to resume local playback.");
      }
      const payload = await postJson<LocalPlaybackCommand>(
        `/sessions/${encodeURIComponent(sessionId)}/queue/play_from`,
        { track_id: replayTrackId }
      );
      await applyLocalPlayback(payload);
      if (audioRef.current && seekMs && seekMs > 0) {
        const resumeAt = seekMs / 1000;
        const player = audioRef.current;
        const applySeek = () => {
          player.currentTime = resumeAt;
        };
        if (Number.isFinite(player.duration) && player.duration > 0) {
          applySeek();
        } else {
          const onLoaded = () => {
            player.removeEventListener("loadedmetadata", onLoaded);
            applySeek();
          };
          player.addEventListener("loadedmetadata", onLoaded);
        }
      }
      return true;
    },
    [applyLocalPlayback, isLocalSession, sessionId]
  );

  useEffect(() => {
    if (!isLocalSession) return;
    const audio = audioRef.current;
    if (!audio) return;
    const onTimeUpdate = () => updateLocalStatusFromAudio();
    const onPause = () => updateLocalStatusFromAudio();
    const onPlay = () => updateLocalStatusFromAudio();
    const onDurationChange = () => updateLocalStatusFromAudio();
    const onEnded = () => {
      requestLocalCommand("/queue/next")
        .then((payload) => applyLocalPlayback(payload))
        .catch((err) => reportError((err as Error).message));
    };
    audio.addEventListener("timeupdate", onTimeUpdate);
    audio.addEventListener("pause", onPause);
    audio.addEventListener("play", onPlay);
    audio.addEventListener("durationchange", onDurationChange);
    audio.addEventListener("ended", onEnded);
    return () => {
      audio.removeEventListener("timeupdate", onTimeUpdate);
      audio.removeEventListener("pause", onPause);
      audio.removeEventListener("play", onPlay);
      audio.removeEventListener("durationchange", onDurationChange);
      audio.removeEventListener("ended", onEnded);
    };
  }, [applyLocalPlayback, isLocalSession, reportError, requestLocalCommand, updateLocalStatusFromAudio]);

  useEffect(() => {
    if (!sessionId || (!activeOutputId && !isLocalSession)) {
      setStatus(null);
    }
  }, [sessionId, activeOutputId, isLocalSession, setStatus]);

  useEffect(() => {
    if (!isLocalSession || !sessionId) return;
    const currentTrackId = localTrackIdRef.current;
    if (!currentTrackId) return;
    saveLocalPlaybackSnapshot(sessionId, {
      track_id: currentTrackId,
      paused: Boolean(status?.paused ?? true),
      elapsed_ms: status?.elapsed_ms ?? null,
      duration_ms: status?.duration_ms ?? null,
      title: status?.title ?? null,
      artist: status?.artist ?? null,
      album: status?.album ?? null,
      saved_at_ms: Date.now()
    });
  }, [
    isLocalSession,
    sessionId,
    status?.album,
    status?.artist,
    status?.duration_ms,
    status?.elapsed_ms,
    status?.paused,
    status?.title
  ]);

  useEffect(() => {
    if (!isLocalSession) return;
    const nowPlayingTrackId = status?.now_playing_track_id ?? null;
    if (!nowPlayingTrackId) return;
    const queueTrack = queue.find((item) => item.kind === "track" && item.id === nowPlayingTrackId);
    if (!queueTrack || queueTrack.kind !== "track") return;

    const nextTitle = queueTrack.title ?? queueTrack.file_name;
    const nextArtist = queueTrack.artist ?? null;
    const nextAlbum = queueTrack.album ?? null;
    if (
      status?.title === nextTitle &&
      (status?.artist ?? null) === nextArtist &&
      (status?.album ?? null) === nextAlbum
    ) {
      return;
    }
    setStatus((prev) =>
      prev
        ? {
            ...prev,
            title: nextTitle,
            artist: nextArtist,
            album: nextAlbum
          }
        : prev
    );
  }, [isLocalSession, queue, setStatus, status?.album, status?.artist, status?.now_playing_track_id, status?.title]);

  useEffect(() => {
    if (!isLocalSession || !sessionId) return;
    const currentQueueItem = queue.find((item) => item.kind === "track" && item.now_playing);
    if (!currentQueueItem || currentQueueItem.kind !== "track") return;
    if (status?.now_playing_track_id === currentQueueItem.id) return;

    const snapshot = loadLocalPlaybackSnapshot(sessionId);
    const title = currentQueueItem.title ?? currentQueueItem.file_name;
    const artist = currentQueueItem.artist ?? null;
    const album = currentQueueItem.album ?? null;
    const elapsedMs = snapshot?.track_id === currentQueueItem.id ? (snapshot.elapsed_ms ?? null) : null;
    const durationMs =
      snapshot?.track_id === currentQueueItem.id
        ? (snapshot.duration_ms ?? currentQueueItem.duration_ms ?? null)
        : (currentQueueItem.duration_ms ?? null);

    localTrackIdRef.current = currentQueueItem.id;
    setStatus((prev) => ({
      ...(prev ?? {}),
      now_playing_track_id: currentQueueItem.id,
      paused: true,
      elapsed_ms: elapsedMs,
      duration_ms: durationMs,
      title,
      artist,
      album
    }));
    markUpdatedAt();
  }, [isLocalSession, markUpdatedAt, queue, sessionId, setStatus, status?.now_playing_track_id]);

  useEffect(() => {
    if (!isLocalSession) return;
    const hasLocalNowPlaying = queue.some((item) => item.kind === "track" && item.now_playing);
    if (hasLocalNowPlaying) return;
    if (!status?.now_playing_track_id) return;
    setStatus(null);
    localTrackIdRef.current = null;
  }, [isLocalSession, queue, setStatus, status?.now_playing_track_id]);

  return {
    audioRef,
    applyLocalPlayback,
    requestLocalCommand,
    toggleLocalPause,
    resumeLocalFromStatus
  };
}
