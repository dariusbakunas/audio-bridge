import { useCallback, useEffect, useRef, useState } from "react";

export type SettingsSection = "metadata" | "logs" | "connection" | "outputs";

export type ViewState = {
  view: "albums" | "album" | "settings";
  albumId?: number | null;
  settingsSection?: SettingsSection;
};

type BrowserViewHistoryState = {
  kind: "audio_hub_view";
  view: ViewState;
};

function sameViewState(a: ViewState, b: ViewState): boolean {
  return (
    a.view === b.view &&
    (a.albumId ?? null) === (b.albumId ?? null) &&
    (a.settingsSection ?? null) === (b.settingsSection ?? null)
  );
}

function toBrowserHistoryState(view: ViewState): BrowserViewHistoryState {
  return {
    kind: "audio_hub_view",
    view
  };
}

function parseBrowserHistoryState(value: unknown): ViewState | null {
  if (!value || typeof value !== "object") return null;
  const state = value as Partial<BrowserViewHistoryState>;
  if (state.kind !== "audio_hub_view" || !state.view) return null;
  const view = state.view;
  if (view.view !== "albums" && view.view !== "album" && view.view !== "settings") return null;
  return {
    view: view.view,
    albumId: view.albumId ?? null,
    settingsSection: view.settingsSection ?? "metadata"
  };
}

type UseViewNavigationArgs = {
  setSettingsOpen: (value: boolean) => void;
  setAlbumViewId: (value: number | null) => void;
  setSettingsSection: (value: SettingsSection) => void;
};

export function useViewNavigation({
  setSettingsOpen,
  setAlbumViewId,
  setSettingsSection
}: UseViewNavigationArgs) {
  const initialViewState: ViewState = {
    view: "albums",
    albumId: null,
    settingsSection: "metadata"
  };
  const [navState, setNavState] = useState<{ stack: ViewState[]; index: number }>(() => ({
    stack: [initialViewState],
    index: 0
  }));
  const applyingHistoryRef = useRef(false);

  const applyViewState = useCallback(
    (state: ViewState) => {
      applyingHistoryRef.current = true;
      if (state.view === "settings") {
        setSettingsSection(state.settingsSection ?? "metadata");
        setSettingsOpen(true);
        setAlbumViewId(null);
        return;
      }
      setSettingsOpen(false);
      if (state.view === "album") {
        setAlbumViewId(state.albumId ?? null);
        return;
      }
      setAlbumViewId(null);
    },
    [setAlbumViewId, setSettingsOpen, setSettingsSection]
  );

  useEffect(() => {
    if (applyingHistoryRef.current) {
      applyingHistoryRef.current = false;
    }
  });

  const pushViewState = useCallback((next: ViewState) => {
    setNavState((prev) => {
      const base = prev.stack.slice(0, prev.index + 1);
      const last = base[base.length - 1];
      const isSame = sameViewState(last, next);
      if (isSame) return prev;
      const stack = [...base, next];
      return { stack, index: stack.length - 1 };
    });
    try {
      window.history.pushState(toBrowserHistoryState(next), "");
    } catch {
      // ignore history failures
    }
  }, []);

  const navigateTo = useCallback(
    (next: ViewState) => {
      applyViewState(next);
      pushViewState(next);
    },
    [applyViewState, pushViewState]
  );

  const canGoBack = navState.index > 0;
  const canGoForward = navState.index < navState.stack.length - 1;

  const goBack = useCallback(() => {
    if (!canGoBack) return;
    window.history.back();
  }, [canGoBack]);

  const goForward = useCallback(() => {
    if (!canGoForward) return;
    window.history.forward();
  }, [canGoForward]);

  useEffect(() => {
    try {
      const existing = parseBrowserHistoryState(window.history.state);
      if (existing) {
        applyViewState(existing);
        setNavState((prev) => {
          const idx = prev.stack.findIndex((item) => sameViewState(item, existing));
          if (idx >= 0) return { ...prev, index: idx };
          return { stack: [existing], index: 0 };
        });
      } else {
        const current = navState.stack[navState.index] ?? initialViewState;
        window.history.replaceState(toBrowserHistoryState(current), "");
      }
    } catch {
      // ignore history failures
    }
    // Intentionally mount-only: initialize browser history state once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const handlePopState = (event: PopStateEvent) => {
      const next = parseBrowserHistoryState(event.state);
      if (!next) {
        return;
      }
      applyViewState(next);
      setNavState((prev) => {
        let index = -1;
        for (let i = prev.index - 1; i >= 0; i -= 1) {
          if (sameViewState(prev.stack[i], next)) {
            index = i;
            break;
          }
        }
        if (index < 0) {
          for (let i = prev.index + 1; i < prev.stack.length; i += 1) {
            if (sameViewState(prev.stack[i], next)) {
              index = i;
              break;
            }
          }
        }
        if (index >= 0) {
          return { ...prev, index };
        }
        const base = prev.stack.slice(0, prev.index + 1);
        const stack = [...base, next];
        return { stack, index: stack.length - 1 };
      });
    };

    window.addEventListener("popstate", handlePopState);
    return () => {
      window.removeEventListener("popstate", handlePopState);
    };
  }, [applyViewState]);

  return {
    navigateTo,
    canGoBack,
    canGoForward,
    goBack,
    goForward
  };
}
