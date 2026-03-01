import { useCallback, useEffect, useState } from "react";

const TRACK_MENU_GAP_PX = 4;
const TRACK_MENU_MARGIN_PX = 8;
const TRACK_MENU_MIN_WIDTH_PX = 220;
const TRACK_MENU_ESTIMATED_HEIGHT_PX = 320;

export function useTrackMenu() {
  const [trackMenuTrackId, setTrackMenuTrackId] = useState<number | null>(null);
  const [trackMenuPosition, setTrackMenuPosition] = useState<{
    top: number;
    right: number;
    up: boolean;
  } | null>(null);

  const closeTrackMenu = useCallback(() => {
    setTrackMenuTrackId(null);
    setTrackMenuPosition(null);
  }, []);

  const toggleTrackMenu = useCallback(
    (trackId: number, target: Element) => {
      if (trackMenuTrackId === trackId) {
        closeTrackMenu();
        return;
      }
      const rect = target.getBoundingClientRect();
      const playerBarTop =
        document.querySelector(".player-bar")?.getBoundingClientRect().top ?? window.innerHeight;
      const bottomLimit = Math.min(window.innerHeight, playerBarTop - TRACK_MENU_MARGIN_PX);
      const minTop = TRACK_MENU_MARGIN_PX;
      const spaceBelow = bottomLimit - rect.bottom;
      const placeAbove = spaceBelow < TRACK_MENU_ESTIMATED_HEIGHT_PX;
      const top = placeAbove
        ? Math.max(
            minTop + TRACK_MENU_ESTIMATED_HEIGHT_PX,
            Math.min(rect.top - TRACK_MENU_GAP_PX, bottomLimit)
          )
        : Math.max(
            minTop,
            Math.min(rect.bottom + TRACK_MENU_GAP_PX, bottomLimit - TRACK_MENU_ESTIMATED_HEIGHT_PX)
          );
      const maxRight = Math.max(
        TRACK_MENU_MARGIN_PX,
        window.innerWidth - TRACK_MENU_MIN_WIDTH_PX - TRACK_MENU_MARGIN_PX
      );
      const unclampedRight = window.innerWidth - rect.right;
      const right = Math.min(Math.max(unclampedRight, TRACK_MENU_MARGIN_PX), maxRight);
      setTrackMenuPosition({
        top,
        right,
        up: placeAbove
      });
      setTrackMenuTrackId(trackId);
    },
    [closeTrackMenu, trackMenuTrackId]
  );

  const runTrackMenuAction = useCallback(
    (action: (trackId: number) => void | Promise<void>, trackId: number) => {
      action(trackId);
      closeTrackMenu();
    },
    [closeTrackMenu]
  );

  useEffect(() => {
    if (!trackMenuTrackId) return;

    function handleDocumentClick(event: MouseEvent) {
      const target = event.target as Element | null;
      if (target?.closest('[data-track-menu="true"]')) {
        return;
      }
      closeTrackMenu();
    }

    document.addEventListener("click", handleDocumentClick);
    return () => {
      document.removeEventListener("click", handleDocumentClick);
    };
  }, [trackMenuTrackId, closeTrackMenu]);

  return {
    trackMenuTrackId,
    trackMenuPosition,
    toggleTrackMenu,
    runTrackMenuAction
  };
}
