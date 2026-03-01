import { useCallback, useEffect, useState } from "react";

import { fetchJson, postJson } from "../api";
import {
  SessionCreateResponse,
  SessionDetailResponse,
  SessionLockInfo,
  SessionLocksResponse,
  SessionSummary,
  SessionsListResponse
} from "../types";

type UseSessionsStateArgs = {
  serverConnected: boolean;
  apiBaseOverride: string;
  appVersion: string;
  getClientId: () => string;
  sessionStorageKey: string;
  onError: (message: string) => void;
};

const WEB_DEFAULT_SESSION_NAME = "Default";
const WEB_DEFAULT_REMOTE_CLIENT_ID = "web-default-global";

function isDefaultSessionName(name: string | null | undefined): boolean {
  return (name ?? "").trim().toLowerCase() === WEB_DEFAULT_SESSION_NAME.toLowerCase();
}

export function useSessionsState({
  serverConnected,
  apiBaseOverride,
  appVersion,
  getClientId,
  sessionStorageKey,
  onError
}: UseSessionsStateArgs) {
  const [sessionId, setSessionId] = useState<string | null>(() => {
    try {
      return localStorage.getItem(sessionStorageKey);
    } catch {
      return null;
    }
  });
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionOutputLocks, setSessionOutputLocks] = useState<SessionLockInfo[]>([]);
  const [sessionBridgeLocks, setSessionBridgeLocks] = useState<SessionLockInfo[]>([]);
  const [activeOutputId, setActiveOutputId] = useState<string | null>(null);

  const refreshSessions = useCallback(async (): Promise<SessionSummary[]> => {
    const clientId = getClientId();
    const response = await fetchJson<SessionsListResponse>(
      `/sessions?client_id=${encodeURIComponent(clientId)}`
    );
    const next = response.sessions ?? [];
    setSessions(next);
    return next;
  }, [getClientId]);

  const refreshSessionLocks = useCallback(async () => {
    const response = await fetchJson<SessionLocksResponse>("/sessions/locks");
    setSessionOutputLocks(response.output_locks ?? []);
    setSessionBridgeLocks(response.bridge_locks ?? []);
  }, []);

  const refreshSessionDetail = useCallback(
    async (id: string) => {
      const clientId = getClientId();
      const detail = await fetchJson<SessionDetailResponse>(
        `/sessions/${encodeURIComponent(id)}?client_id=${encodeURIComponent(clientId)}`
      );
      setActiveOutputId(detail.active_output_id ?? null);
    },
    [getClientId]
  );

  const persistSessionId = useCallback(
    (id: string | null) => {
      try {
        if (id) {
          localStorage.setItem(sessionStorageKey, id);
        } else {
          localStorage.removeItem(sessionStorageKey);
        }
      } catch {
        // ignore storage failures
      }
    },
    [sessionStorageKey]
  );

  const selectSession = useCallback(
    async (nextSessionId: string) => {
      setSessionId(nextSessionId);
      setActiveOutputId(null);
      persistSessionId(nextSessionId);
      await refreshSessionDetail(nextSessionId);
    },
    [persistSessionId, refreshSessionDetail]
  );

  const ensureSession = useCallback(async () => {
    const clientId = getClientId();
    const defaultSession = await postJson<SessionCreateResponse>("/sessions", {
      name: WEB_DEFAULT_SESSION_NAME,
      mode: "remote",
      client_id: WEB_DEFAULT_REMOTE_CLIENT_ID,
      app_version: appVersion,
      owner: "web-ui",
      lease_ttl_sec: 0
    });
    await postJson<SessionCreateResponse>("/sessions", {
      name: "Local",
      mode: "local",
      client_id: clientId,
      app_version: appVersion,
      owner: "web-ui",
      lease_ttl_sec: 0
    });

    const sessionsResponse = await fetchJson<SessionsListResponse>(
      `/sessions?client_id=${encodeURIComponent(clientId)}`
    );
    const nextSessions = sessionsResponse.sessions ?? [];
    setSessions(nextSessions);
    await refreshSessionLocks();

    const stored = (() => {
      try {
        return localStorage.getItem(sessionStorageKey);
      } catch {
        return null;
      }
    })();
    const nextSessionId =
      (stored && nextSessions.some((session) => session.id === stored) ? stored : null) ??
      nextSessions.find((session) => isDefaultSessionName(session.name))?.id ??
      defaultSession.session_id;

    setSessionId(nextSessionId);
    persistSessionId(nextSessionId);
    await refreshSessionDetail(nextSessionId);
  }, [appVersion, getClientId, persistSessionId, refreshSessionDetail, refreshSessionLocks, sessionStorageKey]);

  useEffect(() => {
    if (!serverConnected) return;
    ensureSession().catch((err) => {
      onError((err as Error).message);
    });
  }, [apiBaseOverride, ensureSession, onError, serverConnected]);

  useEffect(() => {
    if (!serverConnected || !sessionId) return;
    refreshSessionDetail(sessionId).catch(() => {
      // Session may have expired or been removed.
    });
  }, [refreshSessionDetail, serverConnected, sessionId]);

  useEffect(() => {
    if (!sessionId) return;
    const current = sessions.find((item) => item.id === sessionId);
    if (!current) return;
    const nextActiveOutputId = current.active_output_id ?? null;
    setActiveOutputId((prev) => (prev === nextActiveOutputId ? prev : nextActiveOutputId));
  }, [sessionId, sessions]);

  useEffect(() => {
    if (!serverConnected || !sessionId) return;
    const sendHeartbeat = async () => {
      try {
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/heartbeat`, {
          state: document.hidden ? "background" : "foreground"
        });
      } catch {
        // Best-effort heartbeat.
      }
    };
    sendHeartbeat();
    const timer = window.setInterval(sendHeartbeat, 10000);
    const onVisibilityChange = () => {
      sendHeartbeat();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [serverConnected, sessionId]);

  useEffect(() => {
    if (!serverConnected) return;
    let mounted = true;
    const poll = async () => {
      try {
        const [sessionsResponse, locksResponse] = await Promise.all([
          fetchJson<SessionsListResponse>(
            `/sessions?client_id=${encodeURIComponent(getClientId())}`
          ),
          fetchJson<SessionLocksResponse>("/sessions/locks")
        ]);
        if (!mounted) return;
        setSessions(sessionsResponse.sessions ?? []);
        setSessionOutputLocks(locksResponse.output_locks ?? []);
        setSessionBridgeLocks(locksResponse.bridge_locks ?? []);
      } catch {
        // Best-effort list refresh.
      }
    };
    poll();
    const timer = window.setInterval(poll, 15000);
    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, [getClientId, serverConnected]);

  return {
    sessionId,
    setSessionId,
    sessions,
    sessionOutputLocks,
    sessionBridgeLocks,
    activeOutputId,
    setActiveOutputId,
    refreshSessions,
    refreshSessionLocks,
    refreshSessionDetail,
    selectSession
  };
}
