import { useCallback, useEffect, useRef, useState } from "react";

import { fetchJson, postJson } from "../api";
import { SessionVolumeResponse } from "../types";

type UseSessionVolumeControlArgs = {
  sessionId: string | null;
  isLocalSession: boolean;
  activeOutputId: string | null;
  canControlVolume: boolean;
  reportError: (message: string) => void;
};

export function useSessionVolumeControl({
  sessionId,
  isLocalSession,
  activeOutputId,
  canControlVolume,
  reportError
}: UseSessionVolumeControlArgs) {
  const [sessionVolume, setSessionVolume] = useState<SessionVolumeResponse | null>(null);
  const [volumeBusy, setVolumeBusy] = useState(false);
  const requestSeqRef = useRef(0);

  const refreshSessionVolume = useCallback(
    async (id: string, silent = true) => {
      try {
        const volume = await fetchJson<SessionVolumeResponse>(
          `/sessions/${encodeURIComponent(id)}/volume`
        );
        setSessionVolume(volume);
      } catch (err) {
        setSessionVolume(null);
        if (!silent) {
          reportError((err as Error).message);
        }
      }
    },
    [reportError]
  );

  useEffect(() => {
    if (!canControlVolume || !sessionId) {
      setSessionVolume(null);
      return;
    }
    requestSeqRef.current += 1;
    void refreshSessionVolume(sessionId, true);
  }, [activeOutputId, canControlVolume, refreshSessionVolume, sessionId]);

  const handleSetVolume = useCallback(
    async (value: number) => {
      if (!sessionId || isLocalSession || !activeOutputId) return;
      const clamped = Math.max(0, Math.min(100, Math.round(value)));
      setSessionVolume((prev) => ({
        value: clamped,
        muted: prev?.muted ?? false,
        source: prev?.source ?? "bridge",
        available: true
      }));
      const requestSeq = ++requestSeqRef.current;
      try {
        const payload = await postJson<SessionVolumeResponse>(
          `/sessions/${encodeURIComponent(sessionId)}/volume`,
          { value: clamped }
        );
        if (requestSeq === requestSeqRef.current) {
          setSessionVolume(payload);
        }
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) {
          return;
        }
        reportError((err as Error).message);
        await refreshSessionVolume(sessionId, true);
      }
    },
    [activeOutputId, isLocalSession, refreshSessionVolume, reportError, sessionId]
  );

  const handleToggleMute = useCallback(async () => {
    if (!sessionId || isLocalSession || !activeOutputId || !sessionVolume) return;
    const nextMuted = !Boolean(sessionVolume.muted);
    setVolumeBusy(true);
    setSessionVolume({ ...sessionVolume, muted: nextMuted });
    try {
      const payload = await postJson<SessionVolumeResponse>(
        `/sessions/${encodeURIComponent(sessionId)}/mute`,
        { muted: nextMuted }
      );
      setSessionVolume(payload);
    } catch (err) {
      reportError((err as Error).message);
      await refreshSessionVolume(sessionId, true);
    } finally {
      setVolumeBusy(false);
    }
  }, [
    activeOutputId,
    isLocalSession,
    refreshSessionVolume,
    reportError,
    sessionId,
    sessionVolume
  ]);

  return {
    sessionVolume,
    setSessionVolume,
    volumeBusy,
    handleSetVolume,
    handleToggleMute
  };
}
