import { useCallback } from "react";

import { QueueItem, StatusResponse, SessionVolumeResponse } from "../types";

type UseSessionContextArgs = {
  setStatus: (value: StatusResponse | null) => void;
  setSessionVolume: (value: SessionVolumeResponse | null) => void;
  setQueue: (value: QueueItem[]) => void;
  setSessionId: (value: string | null) => void;
  setActiveOutputId: (value: string | null) => void;
};

export function useSessionContext({
  setStatus,
  setSessionVolume,
  setQueue,
  setSessionId,
  setActiveOutputId
}: UseSessionContextArgs) {
  const resetSessionContext = useCallback(() => {
    setStatus(null);
    setSessionVolume(null);
    setQueue([]);
  }, [setSessionVolume, setStatus, setQueue]);

  const clearSessionSelection = useCallback(() => {
    setSessionId(null);
    setActiveOutputId(null);
    resetSessionContext();
  }, [resetSessionContext, setActiveOutputId, setSessionId]);

  return {
    resetSessionContext,
    clearSessionSelection
  };
}
