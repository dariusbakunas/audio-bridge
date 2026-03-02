import { useState } from "react";

import { OutputInfo, QueueItem, StatusResponse } from "../types";
import { SettingsSection } from "./useViewNavigation";

type UseAppUiStateArgs = {
  navCollapsedKey: string;
};

export function useAppUiState({ navCollapsedKey }: UseAppUiStateArgs) {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [rescanBusy, setRescanBusy] = useState<boolean>(false);
  const [queueOpen, setQueueOpen] = useState<boolean>(false);
  const [signalOpen, setSignalOpen] = useState<boolean>(false);
  const [outputsOpen, setOutputsOpen] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [catalogOpen, setCatalogOpen] = useState<boolean>(false);
  const [albumNotesOpen, setAlbumNotesOpen] = useState<boolean>(false);
  const [analysisTarget, setAnalysisTarget] = useState<{
    trackId: number;
    title: string;
    artist?: string | null;
  } | null>(null);
  const [navCollapsed, setNavCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(navCollapsedKey) === "1";
    } catch {
      return false;
    }
  });
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("metadata");
  const [albumSearch, setAlbumSearch] = useState<string>("");
  const [albumViewMode, setAlbumViewMode] = useState<"grid" | "list">("grid");
  const [albumViewId, setAlbumViewId] = useState<number | null>(null);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);

  return {
    outputs,
    setOutputs,
    status,
    setStatus,
    queue,
    setQueue,
    rescanBusy,
    setRescanBusy,
    queueOpen,
    setQueueOpen,
    signalOpen,
    setSignalOpen,
    outputsOpen,
    setOutputsOpen,
    settingsOpen,
    setSettingsOpen,
    catalogOpen,
    setCatalogOpen,
    albumNotesOpen,
    setAlbumNotesOpen,
    analysisTarget,
    setAnalysisTarget,
    navCollapsed,
    setNavCollapsed,
    settingsSection,
    setSettingsSection,
    albumSearch,
    setAlbumSearch,
    albumViewMode,
    setAlbumViewMode,
    albumViewId,
    setAlbumViewId,
    updatedAt,
    setUpdatedAt
  };
}
