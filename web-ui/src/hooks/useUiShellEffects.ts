import { MutableRefObject, useEffect } from "react";

type UseUiShellEffectsArgs = {
  navCollapsed: boolean;
  navCollapsedKey: string;
  serverConnected: boolean;
  setAlbumsError: (value: string | null) => void;
  setAlbumTracksError: (value: string | null) => void;
  activeSessionIdRef: MutableRefObject<string | null>;
  sessionId: string | null;
  isLocalSessionRef: MutableRefObject<boolean>;
  isLocalSession: boolean;
  statusNowPlayingTrackId?: number | null;
  signalOpen: boolean;
  setSignalOpen: (value: boolean) => void;
  outputsOpen: boolean;
  setOutputsOpen: (value: boolean) => void;
  albumViewId: number | null;
  setAlbumNotesOpen: (value: boolean) => void;
};

export function useUiShellEffects({
  navCollapsed,
  navCollapsedKey,
  serverConnected,
  setAlbumsError,
  setAlbumTracksError,
  activeSessionIdRef,
  sessionId,
  isLocalSessionRef,
  isLocalSession,
  statusNowPlayingTrackId,
  signalOpen,
  setSignalOpen,
  outputsOpen,
  setOutputsOpen,
  albumViewId,
  setAlbumNotesOpen
}: UseUiShellEffectsArgs) {
  useEffect(() => {
    try {
      localStorage.setItem(navCollapsedKey, navCollapsed ? "1" : "0");
    } catch {
      // ignore storage failures
    }
  }, [navCollapsed, navCollapsedKey]);

  useEffect(() => {
    if (!serverConnected) return;
    setAlbumsError(null);
    setAlbumTracksError(null);
  }, [serverConnected, setAlbumTracksError, setAlbumsError]);

  useEffect(() => {
    activeSessionIdRef.current = sessionId;
    isLocalSessionRef.current = Boolean(isLocalSession);
  }, [activeSessionIdRef, isLocalSession, isLocalSessionRef, sessionId]);

  useEffect(() => {
    if (!statusNowPlayingTrackId && signalOpen) {
      setSignalOpen(false);
    }
  }, [setSignalOpen, signalOpen, statusNowPlayingTrackId]);

  useEffect(() => {
    if (!outputsOpen) return;
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOutputsOpen(false);
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [outputsOpen, setOutputsOpen]);

  useEffect(() => {
    setAlbumNotesOpen(false);
  }, [albumViewId, setAlbumNotesOpen]);

  useEffect(() => {
    if (albumViewId === null) return;
    const main = document.querySelector<HTMLElement>(".main");
    if (main) {
      main.scrollTo({ top: 0, behavior: "smooth" });
    } else {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  }, [albumViewId]);
}
