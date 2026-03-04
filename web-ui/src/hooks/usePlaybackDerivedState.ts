import { useMemo } from "react";

import { QueueItem, SessionSummary, StatusResponse } from "../types";

type UsePlaybackDerivedStateArgs = {
  queue: QueueItem[];
  status: StatusResponse | null;
  isLocalSession: boolean;
  sessionId: string | null;
  activeOutputAvailable: boolean;
  serverConnected: boolean;
  settingsOpen: boolean;
  albumViewId: number | null;
  sessions: SessionSummary[];
  isDefaultSessionName: (name: string | null | undefined) => boolean;
};

export function usePlaybackDerivedState({
  queue,
  status,
  isLocalSession,
  sessionId,
  activeOutputAvailable,
  serverConnected,
  settingsOpen,
  albumViewId,
  sessions,
  isDefaultSessionName
}: UsePlaybackDerivedStateArgs) {
  const queueNowPlayingTrackId = useMemo(() => {
    const item = queue.find(
      (entry) => entry.kind === "track" && Boolean(entry.now_playing) && !Boolean(entry.played)
    );
    return item?.kind === "track" ? item.id : null;
  }, [queue]);

  const replayTrackId = useMemo(() => {
    const playedTracks = queue.filter(
      (entry): entry is QueueItem & { kind: "track" } =>
        entry.kind === "track" && Boolean(entry.played) && Number.isFinite(entry.id)
    );
    if (!playedTracks.length) return null;
    return playedTracks[playedTracks.length - 1].id;
  }, [queue]);

  const hasPlayedHistory = Boolean(replayTrackId);
  // For remote sessions, queue stream is the source of truth for now-playing.
  // This avoids stale title/track state when playback reaches EOF and status lags behind.
  const effectiveNowPlayingTrackId = isLocalSession
    ? queueNowPlayingTrackId ?? status?.now_playing_track_id ?? null
    : queueNowPlayingTrackId;
  const hasNowPlaying = effectiveNowPlayingTrackId !== null;
  const canReplayFromHistory = Boolean(
    sessionId && (isLocalSession || activeOutputAvailable) && !hasNowPlaying && replayTrackId
  );
  const canTogglePlayback = Boolean(
    sessionId && (isLocalSession || activeOutputAvailable) && hasNowPlaying
  );
  const canControlVolume = Boolean(
    serverConnected && sessionId && !isLocalSession && activeOutputAvailable
  );
  const playbackAvailable = Boolean(isLocalSession || activeOutputAvailable);
  const isPlaying = Boolean(playbackAvailable && hasNowPlaying && !status?.paused);
  const isPaused = !isPlaying;

  const viewTitle = settingsOpen ? "Settings" : albumViewId !== null ? "" : "Albums";
  const playButtonTitle = !sessionId
    ? "Creating session..."
    : !activeOutputAvailable && !isLocalSession
    ? isLocalSession
      ? "Local session is ready."
      : "Select an output to control playback."
    : !hasNowPlaying
    ? "Select an album track to play."
    : undefined;

  const showGate = !serverConnected;
  const queueHasNext =
    Boolean(sessionId && (activeOutputAvailable || isLocalSession) && hasNowPlaying) &&
    queue.some((item) => item.kind === "track" && !item.now_playing && !Boolean(item.played));
  const deleteSessionDisabled =
    !serverConnected ||
    !sessionId ||
    sessions.find((item) => item.id === sessionId)?.mode === "local" ||
    isDefaultSessionName(sessions.find((item) => item.id === sessionId)?.name);
  const canGoPrevious = isLocalSession
    ? queue.some((item) => item.kind === "track" && Boolean(item.played))
    : Boolean(activeOutputAvailable && status?.has_previous);

  return {
    queueNowPlayingTrackId,
    replayTrackId,
    effectiveNowPlayingTrackId,
    hasNowPlaying,
    canReplayFromHistory,
    canTogglePlayback,
    canControlVolume,
    isPlaying,
    isPaused,
    viewTitle,
    playButtonTitle,
    showGate,
    queueHasNext,
    deleteSessionDisabled,
    canGoPrevious
  };
}
