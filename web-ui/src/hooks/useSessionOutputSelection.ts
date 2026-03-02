import { useCallback } from "react";

type UseSessionOutputSelectionArgs = {
  isLocalSession: boolean;
  sessionId: string | null;
  handleSelectOutput: (id: string, force?: boolean) => Promise<void>;
  refreshSessions: () => Promise<unknown>;
  refreshSessionLocks: () => Promise<void>;
  refreshSessionDetail: (id: string) => Promise<void>;
};

export function useSessionOutputSelection({
  isLocalSession,
  sessionId,
  handleSelectOutput,
  refreshSessions,
  refreshSessionLocks,
  refreshSessionDetail
}: UseSessionOutputSelectionArgs) {
  return useCallback(
    async (id: string) => {
      if (isLocalSession) return;
      await handleSelectOutput(id, false);
      if (!sessionId) return;
      try {
        await Promise.all([
          refreshSessions(),
          refreshSessionLocks(),
          refreshSessionDetail(sessionId)
        ]);
      } catch {
        // best-effort refresh
      }
    },
    [
      handleSelectOutput,
      isLocalSession,
      refreshSessionDetail,
      refreshSessionLocks,
      refreshSessions,
      sessionId
    ]
  );
}
