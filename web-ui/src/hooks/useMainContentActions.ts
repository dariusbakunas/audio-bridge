import { useCallback } from "react";

import { TrackSummary } from "../types";
import { SettingsSection } from "./useViewNavigation";

type AnalysisTarget = {
  trackId: number;
  title: string;
  artist?: string | null;
} | null;

type UseMainContentActionsArgs = {
  navigateTo: (next: {
    view: "albums" | "album" | "settings";
    albumId?: number | null;
    settingsSection?: SettingsSection;
  }) => void;
  runTrackMenuAction: (action: (id: number) => void, trackId?: number) => void;
  handlePlay: (trackId?: number) => Promise<void>;
  handleQueue: (trackId: number) => Promise<void>;
  handlePlayNext: (trackId: number) => Promise<void>;
  handleRescanTrack: (trackId: number) => Promise<void>;
  openTrackMatchForAlbum: (trackId: number) => void;
  openTrackEditorForAlbum: (trackId: number) => void;
  setAnalysisTarget: (value: AnalysisTarget) => void;
};

export function useMainContentActions({
  navigateTo,
  runTrackMenuAction,
  handlePlay,
  handleQueue,
  handlePlayNext,
  handleRescanTrack,
  openTrackMatchForAlbum,
  openTrackEditorForAlbum,
  setAnalysisTarget
}: UseMainContentActionsArgs) {
  const onSelectAlbum = useCallback(
    (id: number) =>
      navigateTo({
        view: "album",
        albumId: id
      }),
    [navigateTo]
  );

  const onSettingsSectionChange = useCallback(
    (section: SettingsSection) =>
      navigateTo({
        view: "settings",
        settingsSection: section
      }),
    [navigateTo]
  );

  const onMenuPlay = useCallback(
    (trackId: number) =>
      runTrackMenuAction((id) => {
        void handlePlay(id);
      }, trackId),
    [handlePlay, runTrackMenuAction]
  );

  const onMenuQueue = useCallback(
    (trackId: number) =>
      runTrackMenuAction((id) => {
        void handleQueue(id);
      }, trackId),
    [handleQueue, runTrackMenuAction]
  );

  const onMenuPlayNext = useCallback(
    (trackId: number) =>
      runTrackMenuAction((id) => {
        void handlePlayNext(id);
      }, trackId),
    [handlePlayNext, runTrackMenuAction]
  );

  const onMenuRescan = useCallback(
    (trackId: number) =>
      runTrackMenuAction((id) => {
        void handleRescanTrack(id);
      }, trackId),
    [handleRescanTrack, runTrackMenuAction]
  );

  const onFixTrackMatch = useCallback(
    (trackId: number) => runTrackMenuAction(openTrackMatchForAlbum, trackId),
    [openTrackMatchForAlbum, runTrackMenuAction]
  );

  const onEditTrackMetadata = useCallback(
    (trackId: number) => runTrackMenuAction(openTrackEditorForAlbum, trackId),
    [openTrackEditorForAlbum, runTrackMenuAction]
  );

  const onAnalyzeTrack = useCallback(
    (track: TrackSummary) => {
      runTrackMenuAction(() => {
        setAnalysisTarget({
          trackId: track.id,
          title: track.title ?? track.file_name,
          artist: track.artist ?? null
        });
      }, track.id);
    },
    [runTrackMenuAction, setAnalysisTarget]
  );

  return {
    onSelectAlbum,
    onSettingsSectionChange,
    onMenuPlay,
    onMenuQueue,
    onMenuPlayNext,
    onMenuRescan,
    onFixTrackMatch,
    onEditTrackMetadata,
    onAnalyzeTrack
  };
}
