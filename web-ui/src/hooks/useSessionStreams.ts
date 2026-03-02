import { MutableRefObject } from "react";

import { OutputInfo, QueueItem, StatusResponse } from "../types";
import { useOutputsStream, useQueueStream, useStatusStream } from "./streams";

type UseSessionStreamsArgs = {
  serverConnected: boolean;
  streamKey: string;
  sessionId: string | null;
  activeOutputId: string | null;
  isLocalSession: boolean;
  activeSessionIdRef: MutableRefObject<string | null>;
  isLocalSessionRef: MutableRefObject<boolean>;
  setOutputs: (outputs: OutputInfo[]) => void;
  setStatus: (status: StatusResponse | null) => void;
  setQueue: (queue: QueueItem[]) => void;
  setUpdatedAt: (value: Date) => void;
  markServerConnected: () => void;
  refreshSessionDetail: (id: string) => Promise<void>;
  connectionError: (summary: string, endpoint: string) => string;
  reportError: (message: string, severity?: "info" | "warn" | "error") => void;
};

export function useSessionStreams({
  serverConnected,
  streamKey,
  sessionId,
  activeOutputId,
  isLocalSession,
  activeSessionIdRef,
  isLocalSessionRef,
  setOutputs,
  setStatus,
  setQueue,
  setUpdatedAt,
  markServerConnected,
  refreshSessionDetail,
  connectionError,
  reportError
}: UseSessionStreamsArgs) {
  useOutputsStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    onEvent: (data) => {
      setOutputs(data.outputs);
      const sid = activeSessionIdRef.current;
      if (sid) {
        refreshSessionDetail(sid).catch(() => {
          // Best-effort session output sync for cross-client output switches.
        });
      }
      markServerConnected();
    },
    onError: () => {
      const message = connectionError("Live outputs disconnected", "/outputs/stream");
      reportError(message, "warn");
    }
  });

  useStatusStream({
    enabled: serverConnected && !isLocalSession && Boolean(sessionId && activeOutputId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (data) => {
      if (isLocalSessionRef.current) {
        return;
      }
      if (!sessionId || activeSessionIdRef.current !== sessionId) {
        return;
      }
      setStatus(data);
      setUpdatedAt(new Date());
      markServerConnected();
    },
    onError: () => {
      if (!activeOutputId) {
        return;
      }
      const message = connectionError(
        "Live status disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/status/stream`
          : "/sessions/{id}/status/stream"
      );
      reportError(message, "warn");
    }
  });

  useQueueStream({
    enabled: serverConnected && Boolean(sessionId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (items) => {
      setQueue(items ?? []);
      markServerConnected();
    },
    onError: () => {
      const message = connectionError(
        "Live queue disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/queue/stream`
          : "/sessions/{id}/queue/stream"
      );
      reportError(message, "warn");
    }
  });
}
