import { useCallback, useState } from "react";

import { fetchJson, postJson } from "../api";
import { SessionCreateResponse, SessionSummary } from "../types";

type UseSessionUiActionsArgs = {
  sessions: SessionSummary[];
  sessionId: string | null;
  refreshSessions: () => Promise<SessionSummary[]>;
  refreshSessionLocks: () => Promise<void>;
  selectSession: (nextSessionId: string) => Promise<void>;
  reportError: (message: string) => void;
  getClientId: () => string;
  appVersion: string;
  sessionStorageKey: string;
  onSessionContextReset: () => void;
  onNoSessionsRemaining: () => void;
  isDefaultSessionName: (name: string | null | undefined) => boolean;
};

export function useSessionUiActions({
  sessions,
  sessionId,
  refreshSessions,
  refreshSessionLocks,
  selectSession,
  reportError,
  getClientId,
  appVersion,
  sessionStorageKey,
  onSessionContextReset,
  onNoSessionsRemaining,
  isDefaultSessionName
}: UseSessionUiActionsArgs) {
  const [createSessionOpen, setCreateSessionOpen] = useState<boolean>(false);
  const [newSessionName, setNewSessionName] = useState<string>("");
  const [newSessionNeverExpires, setNewSessionNeverExpires] = useState<boolean>(false);
  const [createSessionBusy, setCreateSessionBusy] = useState<boolean>(false);

  const handleSessionChange = useCallback(
    async (nextSessionId: string) => {
      onSessionContextReset();
      try {
        await selectSession(nextSessionId);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [onSessionContextReset, reportError, selectSession]
  );

  const createNamedSession = useCallback(
    async (name: string, neverExpires = false) => {
      try {
        const response = await postJson<SessionCreateResponse>("/sessions", {
          name,
          mode: "remote",
          client_id:
            typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
              ? `${getClientId()}:${crypto.randomUUID()}`
              : `${getClientId()}-${Date.now()}`,
          app_version: appVersion,
          owner: "web-ui",
          ...(neverExpires ? { lease_ttl_sec: 0 } : {})
        });
        await Promise.all([refreshSessions(), refreshSessionLocks()]);
        await handleSessionChange(response.session_id);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [appVersion, getClientId, handleSessionChange, refreshSessionLocks, refreshSessions, reportError]
  );

  const handleCreateSession = useCallback(() => {
    setNewSessionName(`Session ${sessions.length + 1}`);
    setNewSessionNeverExpires(false);
    setCreateSessionOpen(true);
  }, [sessions.length]);

  const submitCreateSession = useCallback(async () => {
    const name = newSessionName.trim();
    if (!name) {
      reportError("Session name is required.");
      return;
    }
    setCreateSessionBusy(true);
    try {
      await createNamedSession(name, newSessionNeverExpires);
      setCreateSessionOpen(false);
      setNewSessionName("");
      setNewSessionNeverExpires(false);
    } finally {
      setCreateSessionBusy(false);
    }
  }, [createNamedSession, newSessionName, newSessionNeverExpires, reportError]);

  const handleDeleteSession = useCallback(async () => {
    if (!sessionId) return;
    const session = sessions.find((item) => item.id === sessionId) ?? null;
    if (!session || isDefaultSessionName(session.name)) {
      return;
    }
    const confirmed = window.confirm(`Delete session "${session.name}"?`);
    if (!confirmed) return;

    try {
      await fetchJson(`/sessions/${encodeURIComponent(sessionId)}`, {
        method: "DELETE"
      });
      const nextSessions = await refreshSessions();
      await refreshSessionLocks();
      const defaultSession =
        nextSessions.find((item) => isDefaultSessionName(item.name)) ?? nextSessions[0] ?? null;
      if (defaultSession) {
        await handleSessionChange(defaultSession.id);
      } else {
        onNoSessionsRemaining();
        try {
          localStorage.removeItem(sessionStorageKey);
        } catch {
          // ignore storage failures
        }
      }
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [
    handleSessionChange,
    isDefaultSessionName,
    onNoSessionsRemaining,
    refreshSessionLocks,
    refreshSessions,
    reportError,
    sessionId,
    sessionStorageKey,
    sessions
  ]);

  return {
    createSessionOpen,
    setCreateSessionOpen,
    newSessionName,
    setNewSessionName,
    newSessionNeverExpires,
    setNewSessionNeverExpires,
    createSessionBusy,
    handleSessionChange,
    handleCreateSession,
    submitCreateSession,
    handleDeleteSession
  };
}
