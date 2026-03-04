import { useCallback } from "react";

import { SettingsSection } from "./useViewNavigation";

type UseAppChromeActionsArgs = {
  navigateTo: (next: {
    view: "albums" | "album" | "settings" | "queue" | "nowPlaying" | "sessions";
    albumId?: number | null;
    settingsSection?: SettingsSection;
  }) => void;
  isLocalSession: boolean;
  compactLayout: boolean;
  setSignalOpen: (value: boolean) => void;
  setQueueOpen: (value: boolean | ((current: boolean) => boolean)) => void;
  setOutputsOpen: (value: boolean) => void;
  handleDeleteSession: () => Promise<void>;
};

export function useAppChromeActions({
  navigateTo,
  isLocalSession,
  compactLayout,
  setSignalOpen,
  setQueueOpen,
  setOutputsOpen,
  handleDeleteSession
}: UseAppChromeActionsArgs) {
  const onAlbumNavigate = useCallback(
    (albumId: number) =>
      navigateTo({
        view: "album",
        albumId
      }),
    [navigateTo]
  );

  const onSignalOpen = useCallback(() => {
    setSignalOpen(true);
  }, [setSignalOpen]);

  const onQueueOpen = useCallback(() => {
    if (compactLayout) {
      setQueueOpen(false);
      navigateTo({
        view: "queue"
      });
      return;
    }
    setQueueOpen((value) => !value);
  }, [compactLayout, navigateTo, setQueueOpen]);

  const onSelectOutput = useCallback(() => {
    if (!isLocalSession) {
      setOutputsOpen(true);
    }
  }, [isLocalSession, setOutputsOpen]);

  const onDeleteSession = useCallback(() => {
    void handleDeleteSession();
  }, [handleDeleteSession]);

  return {
    onAlbumNavigate,
    onSignalOpen,
    onQueueOpen,
    onSelectOutput,
    onDeleteSession
  };
}
