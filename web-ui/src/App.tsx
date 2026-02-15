import { useEffect, useMemo, useState, useCallback, useRef, SetStateAction} from "react";
import { toast, ToastContainer } from "react-toastify";
import {
  Bell,
  ChevronLeft,
  ChevronRight,
  Grid3x3,
  Library,
  List,
  Radio,
  Search,
  Settings
} from "lucide-react";
import {
  apiUrl,
  apiWsUrl,
  fetchJson,
  getDefaultApiBase,
  getEffectiveApiBase,
  getStoredApiBase,
  postJson,
  setStoredApiBase
} from "./api";
import {
  AlbumListResponse,
  AlbumSummary,
  LogEvent,
  MetadataEvent,
  OutputInfo,
  StatusResponse,
  QueueItem,
  TrackResolveResponse,
  TrackListResponse,
  TrackSummary
} from "./types";
import AlbumDetailView from "./components/AlbumDetailView";
import AlbumMetadataDialog from "./components/AlbumMetadataDialog";
import AlbumsView from "./components/AlbumsView";
import MusicBrainzMatchModal from "./components/MusicBrainzMatchModal";
import TrackMetadataModal from "./components/TrackMetadataModal";
import OutputsModal from "./components/OutputsModal";
import PlayerBar from "./components/PlayerBar";
import QueueModal from "./components/QueueModal";
import SettingsView from "./components/SettingsView";
import SignalModal from "./components/SignalModal";
import ConnectionGate from "./components/ConnectionGate";
import {
  useLogsStream,
  useMetadataStream,
  useOutputsStream,
  useQueueStream,
  useStatusStream
} from "./hooks/streams";
import {usePlaybackActions} from "./hooks/usePlaybackActions";

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

interface LogEventEntry {
  id: number;
  event: LogEvent;
}

type MatchTarget = {
  path: string;
  title: string;
  artist: string;
  album?: string | null;
};

type EditTarget = {
  path?: string;
  trackId?: number;
  label: string;
  defaults: {
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    albumArtist?: string | null;
    year?: number | null;
    trackNumber?: number | null;
    discNumber?: number | null;
  };
};

type AlbumEditTarget = {
  albumId: number;
  label: string;
  artist: string;
  defaults: {
    title?: string | null;
    albumArtist?: string | null;
    year?: number | null;
  };
};

type ToastLevel = "error" | "warn" | "info" | "success";

type ToastNotification = {
  id: number;
  level: ToastLevel;
  message: string;
  createdAt: Date;
};

const MAX_METADATA_EVENTS = 200;
const MAX_LOG_EVENTS = 300;

function albumPlaceholder(title?: string | null, artist?: string | null): string {
  const source = title?.trim() || artist?.trim() || "";
  const initials = source
      .split(/\s+/)
      .map((part) => part.replace(/[^A-Za-z0-9]/g, ""))
      .filter(Boolean)
      .map((part) => part[0])
      .join("")
      .slice(0, 2)
      .toUpperCase();
  const label = initials || "NA";
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="#1a1d23"/><stop offset="100%" stop-color="#0f1215"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="18" y="32" font-family="Space Grotesk, sans-serif" font-size="28" fill="#d4965f" text-anchor="start">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

function formatMs(ms?: number | null): string {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatHz(hz?: number | null): string {
  if (!hz) return "—";
  if (hz >= 1000) {
    return `${(hz / 1000).toFixed(1)} kHz`;
  }
  return `${hz} Hz`;
}

function formatRateRange(output: OutputInfo): string {
  if (!output.supported_rates) return "rate range unknown";
  return `${formatHz(output.supported_rates.min_hz)} - ${formatHz(output.supported_rates.max_hz)}`;
}

function normalizeMatch(value?: string | null): string {
  return value?.trim().toLowerCase() ?? "";
}

// Folders view removed.

function describeMetadataEvent(event: MetadataEvent): { title: string; detail?: string } {
  switch (event.kind) {
    case "library_scan_album_start":
      return {title: "Scanning album folder", detail: event.path};
    case "library_scan_album_finish":
      return {title: "Scanned album folder", detail: `${event.tracks} tracks`};
    case "music_brainz_batch":
      return {title: "MusicBrainz batch", detail: `${event.count} candidates`};
    case "music_brainz_lookup_start":
      return {
        title: "MusicBrainz lookup started",
        detail: `${event.title} — ${event.artist}${event.album ? ` (${event.album})` : ""}`
      };
    case "music_brainz_lookup_success":
      return {
        title: "MusicBrainz lookup success",
        detail: event.recording_mbid ?? "match found"
      };
    case "music_brainz_lookup_no_match":
      return {
        title: "MusicBrainz lookup no match",
        detail: `${event.title} — ${event.artist}${event.album ? ` (${event.album})` : ""}`
      };
    case "music_brainz_lookup_failure":
      return {title: "MusicBrainz lookup failed", detail: event.error};
    case "cover_art_batch":
      return {title: "Cover art batch", detail: `${event.count} albums`};
    case "cover_art_fetch_start":
      return {title: "Cover art fetch started", detail: `album ${event.album_id}`};
    case "cover_art_fetch_success":
      return {title: "Cover art fetched", detail: `album ${event.album_id}`};
    case "cover_art_fetch_failure":
      return {
        title: "Cover art fetch failed",
        detail: `${event.error} (attempt ${event.attempts})`
      };
    default:
      return {title: "Metadata event"};
  }
}

function metadataDetailLines(event: MetadataEvent): string[] {
  if (event.kind !== "music_brainz_lookup_no_match") {
    if (event.kind === "library_scan_album_finish") {
      return [event.path];
    }
    if (event.kind === "library_scan_album_start") {
      return [event.path];
    }
    if (event.kind === "cover_art_fetch_failure") {
      return [`MBID: ${event.mbid}`];
    }
    return [];
  }
  const lines: string[] = [];
  if (event.query) {
    lines.push(`Query: ${event.query}`);
  }
  if (event.top_score !== null && event.top_score !== undefined) {
    lines.push(`Top score: ${event.top_score}`);
  }
  if (event.best_recording_title || event.best_recording_id) {
    const title = event.best_recording_title ?? "unknown";
    const id = event.best_recording_id ? ` (${event.best_recording_id})` : "";
    lines.push(`Best: ${title}${id}`);
  }
  return lines;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

export default function App() {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [activeOutputId, setActiveOutputId] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [rescanBusy, setRescanBusy] = useState<boolean>(false);
  const [trackMenuPath, setTrackMenuPath] = useState<string | null>(null);
  const [trackMenuPosition, setTrackMenuPosition] = useState<{
    top: number;
    right: number;
  } | null>(null);
  const [queueOpen, setQueueOpen] = useState<boolean>(false);
  const [signalOpen, setSignalOpen] = useState<boolean>(false);
  const [outputsOpen, setOutputsOpen] = useState<boolean>(false);
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [settingsSection, setSettingsSection] = useState<"metadata" | "logs" | "connection">("metadata");
  const [metadataEvents, setMetadataEvents] = useState<MetadataEventEntry[]>([]);
  const [logEvents, setLogEvents] = useState<LogEventEntry[]>([]);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [albums, setAlbums] = useState<AlbumSummary[]>([]);
  const [albumsLoading, setAlbumsLoading] = useState<boolean>(false);
  const [albumsError, setAlbumsError] = useState<string | null>(null);
  const [albumSearch, setAlbumSearch] = useState<string>("");
  const [albumViewMode, setAlbumViewMode] = useState<"grid" | "list">("grid");
  const [albumViewId, setAlbumViewId] = useState<number | null>(null);
  const [albumTracks, setAlbumTracks] = useState<TrackSummary[]>([]);
  const [albumTracksLoading, setAlbumTracksLoading] = useState<boolean>(false);
  const [albumTracksError, setAlbumTracksError] = useState<string | null>(null);
  const [nowPlayingCover, setNowPlayingCover] = useState<string | null>(null);
  const [nowPlayingCoverFailed, setNowPlayingCoverFailed] = useState<boolean>(false);
  const [nowPlayingAlbumId, setNowPlayingAlbumId] = useState<number | null>(null);
  const [notifications, setNotifications] = useState<ToastNotification[]>([]);
  const [notificationsOpen, setNotificationsOpen] = useState<boolean>(false);
  const [unreadCount, setUnreadCount] = useState<number>(0);
  const [serverConnected, setServerConnected] = useState<boolean>(false);
  const [serverConnecting, setServerConnecting] = useState<boolean>(true);
  const [serverError, setServerError] = useState<string | null>(null);
  const albumsReloadTimerRef = useRef<number | null>(null);
  const albumsReloadQueuedRef = useRef(false);
  const albumsLoadingRef = useRef(false);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const [matchTarget, setMatchTarget] = useState<MatchTarget | null>(null);
  const [editTarget, setEditTarget] = useState<EditTarget | null>(null);
  const [albumEditTarget, setAlbumEditTarget] = useState<AlbumEditTarget | null>(null);
  const logIdRef = useRef(0);
  const metadataIdRef = useRef(0);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const browserWsRef = useRef<WebSocket | null>(null);
  const browserSessionIdRef = useRef<string | null>(null);
  const browserPathRef = useRef<string | null>(null);
  const lastBrowserStatusSentRef = useRef<number>(0);
  const notificationIdRef = useRef(0);
  const toastLastRef = useRef<{ message: string; level: ToastLevel; at: number } | null>(null);

  const closeTrackMenu = useCallback(() => {
    setTrackMenuPath(null);
    setTrackMenuPosition(null);
  }, []);
  const toggleTrackMenu = useCallback(
      (path: string, target: Element) => {
        if (trackMenuPath === path) {
          closeTrackMenu();
          return;
        }
        const rect = target.getBoundingClientRect();
        setTrackMenuPosition({
          top: rect.bottom + 6,
          right: window.innerWidth - rect.right
        });
        setTrackMenuPath(path);
      },
      [trackMenuPath, closeTrackMenu]
  );
  const runTrackMenuAction = useCallback(
      (action: (path: string) => void | Promise<void>, path: string) => {
        action(path);
        closeTrackMenu();
      },
      [closeTrackMenu]
  );
  const handleClearLogs = useCallback(async () => {
    setLogEvents([]);
    try {
      await postJson<{ cleared_at_ms: number }>("/logs/clear");
      setLogsError(null);
    } catch (err) {
      setLogsError((err as Error).message);
    }
  }, []);
  const activeOutput = useMemo(
      () => outputs.find((output) => output.id === activeOutputId) ?? null,
      [outputs, activeOutputId]
  );
  const canTogglePlayback = Boolean(activeOutputId && status?.now_playing);
  const isPlaying = Boolean(status?.now_playing && !status?.paused);
  const isPaused = Boolean(status?.now_playing && status?.paused);
  const uiBuildId = useMemo(() => {
    if (__BUILD_MODE__ === "development") {
      return "dev";
    }
    return `v${__APP_VERSION__}+${__GIT_SHA__}`;
  }, []);
  type ViewState = {
    view: "albums" | "album" | "settings";
    albumId?: number | null;
    settingsSection?: "metadata" | "logs" | "connection";
  };

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

  const viewTitle = settingsOpen ? "Settings" : albumViewId !== null ? "" : "Albums";
  const playButtonTitle = !activeOutputId
    ? "Select an output to control playback."
    : !status?.now_playing
      ? "Select an album track to play."
      : undefined;
  const selectedAlbum = useMemo(
      () => albums.find((album) => album.id === albumViewId) ?? null,
      [albums, albumViewId]
  );
  const filteredAlbums = useMemo(() => {
    const query = albumSearch.trim().toLowerCase();
    if (!query) return albums;
    return albums.filter((album) => {
      const title = album.title?.toLowerCase() ?? "";
      const artist = album.artist?.toLowerCase() ?? "";
      const year = album.year ? String(album.year) : "";
      return title.includes(query) || artist.includes(query) || year.includes(query);
    });
  }, [albums, albumSearch]);
  const heuristicAlbumId = useMemo(() => {
    const albumKey = normalizeMatch(status?.album);
    if (!albumKey) return null;
    const artistKey = normalizeMatch(status?.artist);
    const allowArtistMismatch = (albumArtist?: string | null) => {
      if (!albumArtist) return true;
      const key = normalizeMatch(albumArtist);
      return key === "various artists" || key === "various" || key === "va";
    };
    const match = albums.find((album) => {
      if (normalizeMatch(album.title) !== albumKey) return false;
      if (!artistKey) return true;
      if (!album.artist) return true;
      if (normalizeMatch(album.artist) === artistKey) return true;
      return allowArtistMismatch(album.artist);
    });
    return match?.id ?? null;
  }, [albums, status?.album, status?.artist]);
  const activeAlbumId = nowPlayingAlbumId ?? heuristicAlbumId;

  const [apiBaseOverride, setApiBaseOverride] = useState<string>(() => getStoredApiBase());
  const apiBaseDefault = useMemo(() => getDefaultApiBase(), []);
  const handleApiBaseChange = useCallback((value: string) => {
    setApiBaseOverride(value);
    setStoredApiBase(value);
    setServerConnecting(true);
  }, []);
  const handleApiBaseReset = useCallback(() => {
    setApiBaseOverride("");
    setStoredApiBase("");
    setServerConnecting(true);
  }, []);
  const connectionError = useCallback((label: string, path?: string) => {
    const base = getEffectiveApiBase();
    const target = base ? base : "current origin";
    const tlsHint = base.startsWith("https://")
      ? " If using HTTPS with a self-signed cert, trust it in Keychain or use mkcert."
      : "";
    const url = path ? apiUrl(path) : null;
    const detail = url ? `${target} (${url})` : target;
    return `${label} (${detail}).${tlsHint}`;
  }, []);

  const markServerConnected = useCallback(() => {
    setServerConnected(true);
    setServerConnecting(false);
    setServerError(null);
  }, []);

  const markServerDisconnected = useCallback((message: string) => {
    setServerConnected(false);
    setServerConnecting(false);
    setServerError(message);
  }, []);

  const pushToast = useCallback((message: string, level: ToastLevel = "error") => {
    const now = Date.now();
    const last = toastLastRef.current;
    if (last && last.message === message && last.level === level && now - last.at < 2500) {
      return;
    }
    toastLastRef.current = { message, level, at: now };
    const id = (notificationIdRef.current += 1);
    const entry: ToastNotification = {
      id,
      level,
      message,
      createdAt: new Date()
    };
    setNotifications((prev) => [entry, ...prev].slice(0, 200));
    setUnreadCount((prev) => prev + 1);
    const toastId = `${level}:${message}`;
    switch (level) {
      case "success":
        toast.success(message, { toastId });
        break;
      case "info":
        toast.info(message, { toastId });
        break;
      case "warn":
        toast.warn(message, { toastId });
        break;
      default:
        toast.error(message, { toastId });
        break;
    }
  }, []);

  const reportError = useCallback(
    (message: string | null, level: ToastLevel = "error") => {
      if (!message) return;
      pushToast(message, level);
    },
    [pushToast]
  );

  const clearNotifications = useCallback(() => {
    setNotifications([]);
    setUnreadCount(0);
  }, []);

  const toggleNotifications = useCallback(() => {
    setNotificationsOpen((prev) => {
      const next = !prev;
      if (next) {
        setUnreadCount(0);
      }
      return next;
    });
  }, []);

  useEffect(() => {
    let active = true;
    let timer: number | null = null;

    const checkHealth = async () => {
      try {
        await fetchJson<{ status: string }>("/health");
        if (!active) return;
        markServerConnected();
        setAlbumsError(null);
        setAlbumTracksError(null);
      } catch (err) {
        if (!active) return;
        const message = connectionError("Hub server not reachable", "/health");
        markServerDisconnected(message);
      }
    };

    checkHealth();
    timer = window.setInterval(checkHealth, 5000);

    return () => {
      active = false;
      if (timer !== null) {
        window.clearInterval(timer);
      }
    };
  }, [apiBaseOverride, connectionError, markServerConnected, markServerDisconnected]);

  useEffect(() => {
    if (notificationsOpen) {
      setUnreadCount(0);
    }
  }, [notificationsOpen]);

  useEffect(() => {
    if (!notificationsOpen) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [notificationsOpen]);

  const openTrackMatchForAlbum = useCallback(
    (path: string) => {
      const track = albumTracks.find((item) => item.path === path);
      const title = track?.title ?? track?.file_name ?? path;
      const artist = track?.artist ?? "Unknown artist";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      setMatchTarget({
        path,
        title,
        artist,
        album
      });
    },
    [albumTracks, selectedAlbum]
  );

  const openAlbumEditor = useCallback(() => {
    if (!selectedAlbum) return;
    const label = selectedAlbum.artist
      ? `${selectedAlbum.title} — ${selectedAlbum.artist}`
      : selectedAlbum.title;
    setAlbumEditTarget({
      albumId: selectedAlbum.id,
      label,
      artist: selectedAlbum.artist ?? "Unknown artist",
      defaults: {
        title: selectedAlbum.title,
        albumArtist: selectedAlbum.artist ?? null,
        year: selectedAlbum.year ?? null
      }
    });
  }, [selectedAlbum]);

  const openTrackEditorForAlbum = useCallback(
    (path: string) => {
      const track = albumTracks.find((item) => item.path === path);
      const title = track?.title ?? track?.file_name ?? path;
      const artist = track?.artist ?? "";
      const album = track?.album ?? selectedAlbum?.title ?? "";
      const label = artist ? `${title} — ${artist}` : title;
      setEditTarget({
        path,
        trackId: track?.id,
        label,
        defaults: {
          title,
          artist,
          album,
          albumArtist: selectedAlbum?.artist ?? null,
          trackNumber: track?.track_number ?? null,
          discNumber: track?.disc_number ?? null
        }
      });
    },
    [albumTracks, selectedAlbum]
  );

  const matchLabel = matchTarget
    ? `${matchTarget.title}${matchTarget.artist ? ` — ${matchTarget.artist}` : ""}`
    : "";
  const matchDefaults = matchTarget
    ? {
        title: matchTarget.title,
        artist: matchTarget.artist,
        album: matchTarget.album ?? ""
      }
    : { title: "", artist: "", album: "" };
  const editLabel = editTarget?.label ?? "";
  const editDefaults = editTarget?.defaults ?? {};
  const albumEditLabel = albumEditTarget?.label ?? "";
  const albumEditDefaults = albumEditTarget?.defaults ?? {};

  const applyViewState = useCallback((state: ViewState) => {
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
  }, []);

  useEffect(() => {
    if (applyingHistoryRef.current) {
      applyingHistoryRef.current = false;
    }
  });

  const pushViewState = useCallback((next: ViewState) => {
    setNavState((prev) => {
      const base = prev.stack.slice(0, prev.index + 1);
      const last = base[base.length - 1];
      const isSame =
        last.view === next.view &&
        (last.albumId ?? null) === (next.albumId ?? null) &&
        (last.settingsSection ?? null) === (next.settingsSection ?? null);
      if (isSame) return prev;
      const stack = [...base, next];
      return { stack, index: stack.length - 1 };
    });
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
    setNavState((prev) => {
      if (prev.index <= 0) return prev;
      const index = prev.index - 1;
      const target = prev.stack[index];
      applyViewState(target);
      return { ...prev, index };
    });
  }, [applyViewState]);

  const goForward = useCallback(() => {
    setNavState((prev) => {
      if (prev.index >= prev.stack.length - 1) return prev;
      const index = prev.index + 1;
      const target = prev.stack[index];
      applyViewState(target);
      return { ...prev, index };
    });
  }, [applyViewState]);

  useEffect(() => {
    if (!trackMenuPath) return;

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
  }, [trackMenuPath, closeTrackMenu]);

  useEffect(() => {
    if (!isPlaying && signalOpen) {
      setSignalOpen(false);
    }
  }, [isPlaying, signalOpen]);

  const streamKey = useMemo(
    () => `${apiBaseOverride}:${serverConnected ? "up" : "down"}`,
    [apiBaseOverride, serverConnected]
  );

  useOutputsStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    onEvent: (data) => {
      const activeId = data.outputs.some((output) => output.id === data.active_id)
          ? data.active_id
          : null;
      setOutputs(data.outputs);
      setActiveOutputId(activeId);
      markServerConnected();
    },
    onError: () => {
      const message = connectionError("Live outputs disconnected", "/outputs/stream");
      reportError(message, "warn");
    }
  });

  const {
    handleRescanLibrary,
    handleRescanTrack,
    handlePause,
    handleNext,
    handleSelectOutput,
    handlePlay,
    handlePlayAlbumTrack,
    handlePlayAlbumById,
    handleQueue,
    handlePlayNext
  } = usePlaybackActions({
    activeOutputId,
    rescanBusy,
    setError: reportError,
    setActiveOutputId,
    setRescanBusy
  });

  useMetadataStream({
    enabled: settingsOpen && serverConnected,
    onEvent: (event) => {
      const entry: MetadataEventEntry = {
        id: (metadataIdRef.current += 1),
        time: new Date(),
        event
      };
      setMetadataEvents((prev) => [entry, ...prev].slice(0, MAX_METADATA_EVENTS));
    },
    onError: () =>
      reportError(connectionError("Live metadata updates disconnected", "/metadata/stream"), "warn")
  });

  useLogsStream({
    enabled: settingsOpen && serverConnected,
    onSnapshot: (items) => {
      const entries = items
          .map((entry) => ({
            id: (logIdRef.current += 1),
            event: entry
          }))
          .reverse()
          .slice(0, MAX_LOG_EVENTS);
      setLogEvents(entries);
      setLogsError(null);
    },
    onEvent: (entry) => {
      const row: LogEventEntry = {
        id: (logIdRef.current += 1),
        event: entry
      };
      setLogEvents((prev) => [row, ...prev].slice(0, MAX_LOG_EVENTS));
    },
    onError: () => {
      const message = connectionError("Live logs disconnected", "/logs/stream");
      setLogsError(message);
      reportError(message, "warn");
    }
  });

  useStatusStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    activeOutputId,
    onEvent: (data: SetStateAction<StatusResponse | null>) => {
      setStatus(data);
      setUpdatedAt(new Date());
      markServerConnected();
    },
    onError: () => {
      const message = connectionError(
        "Live status disconnected",
        activeOutputId ? `/outputs/${encodeURIComponent(activeOutputId)}/status/stream` : undefined
      );
      reportError(message, "warn");
    }
  });

  const sendBrowserStatus = useCallback((force = false) => {
    const ws = browserWsRef.current;
    const audio = audioRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN || !audio) return;
    const now = Date.now();
    if (!force && now - lastBrowserStatusSentRef.current < 500) {
      return;
    }
    lastBrowserStatusSentRef.current = now;
    const hasSrc = Boolean(audio.src);
    const duration = hasSrc && Number.isFinite(audio.duration)
      ? Math.floor(audio.duration * 1000)
      : null;
    const elapsed = hasSrc && Number.isFinite(audio.currentTime)
      ? Math.floor(audio.currentTime * 1000)
      : null;
    ws.send(JSON.stringify({
      type: "status",
      paused: audio.paused,
      elapsed_ms: elapsed,
      duration_ms: duration,
      now_playing: browserPathRef.current
    }));
  }, []);

  const sendBrowserEnded = useCallback(() => {
    const ws = browserWsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ type: "ended" }));
  }, []);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    const handleTimeUpdate = () => sendBrowserStatus();
    const handlePause = () => sendBrowserStatus(true);
    const handlePlay = () => sendBrowserStatus(true);
    const handleDurationChange = () => sendBrowserStatus(true);
    const handleEnded = () => {
      browserPathRef.current = null;
      sendBrowserEnded();
      sendBrowserStatus(true);
    };
    audio.addEventListener("timeupdate", handleTimeUpdate);
    audio.addEventListener("pause", handlePause);
    audio.addEventListener("play", handlePlay);
    audio.addEventListener("durationchange", handleDurationChange);
    audio.addEventListener("ended", handleEnded);
    return () => {
      audio.removeEventListener("timeupdate", handleTimeUpdate);
      audio.removeEventListener("pause", handlePause);
      audio.removeEventListener("play", handlePlay);
      audio.removeEventListener("durationchange", handleDurationChange);
      audio.removeEventListener("ended", handleEnded);
    };
  }, [sendBrowserStatus, sendBrowserEnded]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      sendBrowserStatus();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [sendBrowserStatus]);

  useEffect(() => {
    let mounted = true;
    let retryTimer: number | null = null;
    const connect = () => {
      if (!mounted) return;
      const ws = new WebSocket(apiWsUrl("/browser/ws"));
      browserWsRef.current = ws;
      ws.onopen = () => {
        const name = `Browser (${navigator.platform || "unknown"})`;
        ws.send(JSON.stringify({ type: "hello", name }));
      };
      ws.onmessage = (event) => {
        let payload: any;
        try {
          payload = JSON.parse(event.data);
        } catch {
          return;
        }
        const audio = audioRef.current;
        if (!audio) return;
        switch (payload?.type) {
          case "hello":
            browserSessionIdRef.current = payload.session_id ?? null;
            break;
          case "play": {
            const url = payload.url as string | undefined;
            if (!url) return;
            const startPaused = Boolean(payload.start_paused);
            const seekMs = typeof payload.seek_ms === "number" ? payload.seek_ms : null;
            browserPathRef.current = typeof payload.path === "string" ? payload.path : null;
            const applyStart = () => {
              if (seekMs !== null) {
                audio.currentTime = seekMs / 1000;
              }
              if (startPaused) {
                audio.pause();
              } else {
                audio.play().catch(() => {});
              }
              sendBrowserStatus(true);
            };
            audio.src = url;
            audio.load();
            if (seekMs === null) {
              applyStart();
            } else {
              const onLoaded = () => {
                audio.removeEventListener("loadedmetadata", onLoaded);
                applyStart();
              };
              audio.addEventListener("loadedmetadata", onLoaded);
            }
            break;
          }
          case "pause_toggle":
            if (audio.paused) {
              audio.play().catch(() => {});
            } else {
              audio.pause();
            }
            break;
          case "stop":
            audio.pause();
            audio.removeAttribute("src");
            audio.load();
            browserPathRef.current = null;
            sendBrowserStatus(true);
            break;
          case "seek":
            if (typeof payload.ms === "number") {
              audio.currentTime = payload.ms / 1000;
              sendBrowserStatus(true);
            }
            break;
          default:
            break;
        }
      };
      ws.onerror = () => {
        ws.close();
      };
      ws.onclose = () => {
        if (!mounted) return;
        retryTimer = window.setTimeout(connect, 1500);
      };
    };
    connect();
    return () => {
      mounted = false;
      if (retryTimer !== null) {
        window.clearTimeout(retryTimer);
      }
      browserWsRef.current?.close();
    };
  }, [sendBrowserStatus]);
  useEffect(() => {
    if (!activeOutputId) {
      setStatus(null);
    }
  }, [activeOutputId]);

  useEffect(() => {
    const path = status?.now_playing ?? null;
    if (!path) {
      setNowPlayingCover(null);
      setNowPlayingCoverFailed(false);
      setNowPlayingAlbumId(null);
      return;
    }
    const queueMatch = queue.find(
      (item) => item.kind === "track" && item.path === path && item.id
    ) as { id?: number } | undefined;
    if (queueMatch?.id) {
      setNowPlayingCover(apiUrl(`/tracks/${queueMatch.id}/cover`));
    } else {
      setNowPlayingCover(apiUrl(`/art?path=${encodeURIComponent(path)}`));
    }
    setNowPlayingCoverFailed(false);
    let active = true;
    fetchJson<TrackResolveResponse>(`/tracks/resolve?path=${encodeURIComponent(path)}`)
      .then((response) => {
        if (!active) return;
        setNowPlayingAlbumId(response?.album_id ?? null);
      })
      .catch(() => {
        if (!active) return;
        setNowPlayingAlbumId(null);
      });
    return () => {
      active = false;
    };
  }, [queue, status?.now_playing]);

  useQueueStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    onEvent: (items) => {
      setQueue(items ?? []);
      markServerConnected();
    },
    onError: () => {
      const message = connectionError("Live queue disconnected", "/queue/stream");
      reportError(message, "warn");
    }
  });

  useEffect(() => {
    if (!outputsOpen) return;
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOutputsOpen(false);
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [outputsOpen]);

  // Library list view removed; no directory loading needed.

  const loadAlbums = useCallback(async () => {
    if (!albumsLoadingRef.current) {
      setAlbumsLoading(true);
    }
    albumsLoadingRef.current = true;
    try {
      const response = await fetchJson<AlbumListResponse>("/albums?limit=200");
      setAlbums(response.items ?? []);
      setAlbumsError(null);
      markServerConnected();
    } catch (err) {
      const message = (err as Error).message;
      setAlbumsError(message);
    } finally {
      albumsLoadingRef.current = false;
      setAlbumsLoading(false);
      if (albumsReloadQueuedRef.current) {
        albumsReloadQueuedRef.current = false;
        if (albumsReloadTimerRef.current === null) {
          albumsReloadTimerRef.current = window.setTimeout(() => {
            albumsReloadTimerRef.current = null;
            loadAlbums();
          }, 250);
        }
      }
    }
  }, []);

  const requestAlbumsReload = useCallback(() => {
    if (albumsLoadingRef.current) {
      albumsReloadQueuedRef.current = true;
      return;
    }
    if (albumsReloadTimerRef.current !== null) return;
    albumsReloadTimerRef.current = window.setTimeout(() => {
      albumsReloadTimerRef.current = null;
      loadAlbums();
    }, 250);
  }, [loadAlbums]);

  useEffect(() => {
    if (!serverConnected) return;
    loadAlbums();
  }, [requestAlbumsReload, loadAlbums, serverConnected]);

  useEffect(() => {
    if (!serverConnected) return;
    let mounted = true;
    const stream = new EventSource(apiUrl("/albums/stream"));
    stream.addEventListener("albums", () => {
      if (!mounted) return;
      requestAlbumsReload();
    });
    stream.onerror = () => {
      if (!mounted) return;
      const message = connectionError("Live albums disconnected", "/albums/stream");
      setAlbumsError(message);
    };
    return () => {
      mounted = false;
      stream.close();
    };
  }, [connectionError, requestAlbumsReload, serverConnected, streamKey]);

  const loadAlbumTracks = useCallback(async (albumId: number | null) => {
    if (albumId === null) return;
    setAlbumTracksLoading(true);
    try {
      const response = await fetchJson<TrackListResponse>(
        `/tracks?album_id=${albumId}&limit=500`
      );
      setAlbumTracks(response.items ?? []);
      setAlbumTracksError(null);
      markServerConnected();
    } catch (err) {
      const message = (err as Error).message;
      setAlbumTracksError(message);
    } finally {
      setAlbumTracksLoading(false);
    }
  }, [markServerConnected]);

  useEffect(() => {
    if (!serverConnected) return;
    loadAlbumTracks(albumViewId);
  }, [albumViewId, loadAlbumTracks, serverConnected]);

  const handlePlayMedia = useCallback(async () => {
    if (status?.now_playing) {
      if (status.paused) {
        await handlePause();
      }
      return;
    }
  }, [handlePause, status?.now_playing, status?.paused]);

  const handlePauseMedia = useCallback(async () => {
    if (status?.now_playing && !status?.paused) {
      await handlePause();
    }
  }, [handlePause, status?.now_playing, status?.paused]);

  const handlePreviousMedia = useCallback(async () => {
    try {
      await postJson("/queue/previous");
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [reportError]);

  useEffect(() => {
    function handleKey(event: KeyboardEvent) {
      if (event.code !== "Space") return;
      if (event.repeat) return;
      if (isEditableTarget(event.target)) return;
      event.preventDefault();
      handlePrimaryAction();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [handlePrimaryAction]);

  useEffect(() => {
    const session = navigator.mediaSession;
    if (!session) return;

    if (status?.title || status?.artist || status?.album) {
      const artwork = nowPlayingCover ? [{ src: nowPlayingCover, sizes: "512x512" }] : [];
      session.metadata = new MediaMetadata({
        title: status?.title ?? "",
        artist: status?.artist ?? "",
        album: status?.album ?? "",
        artwork
      });
    } else {
      session.metadata = null;
    }

    try {
      session.setActionHandler("play", () => {
        handlePlayMedia();
      });
      session.setActionHandler("pause", () => {
        handlePauseMedia();
      });
      session.setActionHandler("previoustrack", () => {
        handlePreviousMedia();
      });
      session.setActionHandler("nexttrack", () => {
        handleNext();
      });
    } catch {
      // MediaSession action handlers are best-effort.
    }

    return () => {
      try {
        session.setActionHandler("play", null);
        session.setActionHandler("pause", null);
        session.setActionHandler("previoustrack", null);
        session.setActionHandler("nexttrack", null);
      } catch {
        // Best-effort cleanup.
      }
    };
  }, [
    handleNext,
    handlePauseMedia,
    handlePlayMedia,
    handlePreviousMedia,
    nowPlayingCover,
    status?.album,
    status?.artist,
    status?.title
  ]);

  const handleQueuePlayFrom = useCallback(async (payload: { trackId?: number; path?: string }) => {
    try {
      if (payload.trackId) {
        await postJson("/queue/play_from", { track_id: payload.trackId });
      } else if (payload.path) {
        await postJson("/queue/play_from", { path: payload.path });
      } else {
        throw new Error("Missing track id or path for queue playback.");
      }
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [reportError]);

  async function handlePrimaryAction() {
    if (status?.now_playing) {
      await handlePause();
      return;
    }
  }

  const showGate = !serverConnected;
  return (
    <div className={`app ${settingsOpen ? "settings-mode" : ""} ${showGate ? "has-gate" : ""}`}>
      {showGate ? (
        <ConnectionGate
          status={serverConnecting ? "connecting" : "disconnected"}
          message={serverError}
          apiBase={apiBaseOverride}
          apiBaseDefault={apiBaseDefault}
          onApiBaseChange={handleApiBaseChange}
          onApiBaseReset={handleApiBaseReset}
          onReconnect={() => window.location.reload()}
        />
      ) : null}
      <div className="layout">
        <aside className="side-nav">
          <div className="nav-brand">
            <div className="nav-mark">
              <Radio className="nav-mark-icon" aria-hidden="true" />
            </div>
            <div>
              <div className="nav-title">Audio Hub</div>
              <div className="nav-subtitle">Lossless control with a live signal view.</div>
            </div>
          </div>
          <div className="nav-section">
            <div className="nav-label">Library</div>
            <button
              className={`nav-button ${!settingsOpen ? "active" : ""}`}
              onClick={() =>
                navigateTo({
                  view: "albums"
                })
              }
            >
              <Library className="nav-icon" aria-hidden="true" />
              <span>Albums</span>
            </button>
          </div>
          <div className="nav-section">
            <div className="nav-label">System</div>
            <button
              className={`nav-button ${settingsOpen ? "active" : ""}`}
              onClick={() =>
                navigateTo({
                  view: "settings",
                  settingsSection: "metadata"
                })
              }
            >
              <Settings className="nav-icon" aria-hidden="true" />
              <span>Settings</span>
            </button>
          </div>
        </aside>

        <main className={`main ${showGate ? "disabled" : ""}`}>
          <header className="view-header">
            <div className="view-header-row">
              <div className="view-nav">
                <button
                  className="icon-btn"
                  onClick={goBack}
                  disabled={!canGoBack}
                  aria-label="Back"
                  title="Back"
                  type="button"
                >
                  <ChevronLeft className="icon" aria-hidden="true" />
                </button>
                {canGoForward ? (
                  <button
                    className="icon-btn"
                    onClick={goForward}
                    aria-label="Forward"
                    title="Forward"
                    type="button"
                  >
                    <ChevronRight className="icon" aria-hidden="true" />
                  </button>
                ) : null}
              </div>
              {viewTitle ? <h1>{viewTitle}</h1> : <span />}
              <div className="view-header-actions">
                {!settingsOpen && albumViewId === null ? (
                  <div className="header-tools">
                    <div className="header-search">
                      <Search className="icon" aria-hidden="true" />
                      <input
                        className="header-search-input"
                        type="search"
                        placeholder="Search albums, artists..."
                        value={albumSearch}
                        onChange={(event) => setAlbumSearch(event.target.value)}
                        aria-label="Search albums"
                      />
                    </div>
                    <div className="view-toggle" role="tablist" aria-label="Album view">
                      <button
                        type="button"
                        className={`view-toggle-btn ${albumViewMode === "grid" ? "active" : ""}`}
                        onClick={() => setAlbumViewMode("grid")}
                        aria-pressed={albumViewMode === "grid"}
                        title="Grid view"
                      >
                        <Grid3x3 className="icon" aria-hidden="true" />
                      </button>
                      <button
                        type="button"
                        className={`view-toggle-btn ${albumViewMode === "list" ? "active" : ""}`}
                        onClick={() => setAlbumViewMode("list")}
                        aria-pressed={albumViewMode === "list"}
                        title="List view"
                      >
                        <List className="icon" aria-hidden="true" />
                      </button>
                    </div>
                  </div>
                ) : null}
                <button
                  className={`icon-btn notification-btn ${notificationsOpen ? "active" : ""}`}
                  onClick={toggleNotifications}
                  aria-label="Notifications"
                  title="Notifications"
                  type="button"
                >
                  <Bell className="icon" aria-hidden="true" />
                  {unreadCount > 0 ? (
                    <span className="notification-badge">
                      {unreadCount > 99 ? "99+" : unreadCount}
                    </span>
                  ) : null}
                </button>
              </div>
            </div>
          </header>

          {!settingsOpen && albumViewId === null ? (
            <section className="grid">
              <AlbumsView
                albums={filteredAlbums}
                loading={albumsLoading}
                error={albumsError}
                placeholder={albumPlaceholder}
                canPlay={Boolean(activeOutputId)}
                activeAlbumId={activeAlbumId}
                isPlaying={isPlaying}
                isPaused={isPaused}
                viewMode={albumViewMode}
                onSelectAlbum={(id) =>
                  navigateTo({
                    view: "album",
                    albumId: id
                  })
                }
                onPlayAlbum={handlePlayAlbumById}
                onPause={handlePause}
              />
            </section>
          ) : null}

          {albumViewId !== null && !settingsOpen ? (
            <AlbumDetailView
              album={selectedAlbum}
              tracks={albumTracks}
              loading={albumTracksLoading}
              error={albumTracksError}
              placeholder={albumPlaceholder}
              canPlay={Boolean(activeOutputId) && albumTracks.length > 0}
              activeAlbumId={activeAlbumId}
              isPlaying={isPlaying}
              isPaused={isPaused}
              onPause={handlePause}
              formatMs={formatMs}
              onPlayAlbum={() => {
                if (!selectedAlbum) return;
                handlePlayAlbumById(selectedAlbum.id);
              }}
              onPlayTrack={handlePlayAlbumTrack}
              trackMenuPath={trackMenuPath}
              trackMenuPosition={trackMenuPosition}
              onToggleMenu={toggleTrackMenu}
              onMenuPlay={(path) => runTrackMenuAction(handlePlay, path)}
              onMenuQueue={(path) => runTrackMenuAction(handleQueue, path)}
              onMenuPlayNext={(path) => runTrackMenuAction(handlePlayNext, path)}
              onMenuRescan={(path) => runTrackMenuAction(handleRescanTrack, path)}
              onFixTrackMatch={(path) => runTrackMenuAction(openTrackMatchForAlbum, path)}
              onEditTrackMetadata={(path) =>
                runTrackMenuAction(openTrackEditorForAlbum, path)
              }
              onEditAlbumMetadata={openAlbumEditor}
            />
          ) : null}

          <SettingsView
            active={settingsOpen}
            section={settingsSection}
            onSectionChange={(section) =>
              navigateTo({
                view: "settings",
                settingsSection: section
              })
            }
            apiBase={apiBaseOverride}
            apiBaseDefault={apiBaseDefault}
            onApiBaseChange={handleApiBaseChange}
            onApiBaseReset={handleApiBaseReset}
            onReconnect={() => window.location.reload()}
            metadataEvents={metadataEvents}
            logEvents={logEvents}
            logsError={logsError}
            rescanBusy={rescanBusy}
            onClearMetadata={() => setMetadataEvents([])}
            onRescanLibrary={handleRescanLibrary}
            onClearLogs={handleClearLogs}
            describeMetadataEvent={describeMetadataEvent}
            metadataDetailLines={metadataDetailLines}
          />
        </main>
      </div>

      {!showGate ? (
        <ToastContainer
          position="top-right"
          autoClose={6000}
          newestOnTop
          closeOnClick
          pauseOnFocusLoss
          pauseOnHover
          theme="light"
        />
      ) : null}

      {notificationsOpen && !showGate ? (
        <div className="side-panel-backdrop" onClick={() => setNotificationsOpen(false)}>
          <aside
            className="side-panel notification-panel"
            aria-label="Notifications"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="card-header">
              <span>Notifications</span>
              <div className="card-actions">
                <span className="pill">{notifications.length} items</span>
                <button className="btn ghost small" onClick={clearNotifications}>
                  Clear
                </button>
                <button className="btn ghost small" onClick={() => setNotificationsOpen(false)}>
                  Close
                </button>
              </div>
            </div>
            <div className="notification-list">
              {notifications.length === 0 ? (
                <div className="muted small">No notifications yet.</div>
              ) : null}
              {notifications.map((entry) => (
                <div key={entry.id} className={`notification-item level-${entry.level}`}>
                  <div className="notification-message">{entry.message}</div>
                  <div className="notification-time">{entry.createdAt.toLocaleTimeString()}</div>
                </div>
              ))}
            </div>
          </aside>
        </div>
      ) : null}

      {!showGate ? (
        <PlayerBar
          status={status}
          nowPlayingCover={nowPlayingCover}
          nowPlayingCoverFailed={nowPlayingCoverFailed}
          showSignalPath={isPlaying}
          canTogglePlayback={canTogglePlayback}
          canGoPrevious={Boolean(status?.has_previous)}
          playButtonTitle={playButtonTitle}
          queueHasItems={Boolean(activeOutputId) && queue.length > 0}
          queueOpen={queueOpen}
          activeOutput={activeOutput}
          activeAlbumId={activeAlbumId}
          uiBuildId={uiBuildId}
          formatMs={formatMs}
          placeholderCover={albumPlaceholder(status?.album, status?.artist)}
          onCoverError={() => setNowPlayingCoverFailed(true)}
          onAlbumNavigate={(albumId) =>
            navigateTo({
              view: "album",
              albumId
            })
          }
          onPrimaryAction={handlePrimaryAction}
          onPrevious={handlePreviousMedia}
          onNext={handleNext}
          onSignalOpen={() => setSignalOpen(true)}
          onQueueOpen={() => setQueueOpen((value) => !value)}
          onSelectOutput={() => setOutputsOpen(true)}
        />
      ) : null}

      {!showGate ? (
        <OutputsModal
        open={outputsOpen}
        outputs={outputs}
        activeOutputId={activeOutputId}
        onClose={() => setOutputsOpen(false)}
        onSelectOutput={handleSelectOutput}
        formatRateRange={formatRateRange}
        />
      ) : null}

      {!showGate ? (
        <SignalModal
        open={signalOpen}
        status={status}
        activeOutput={activeOutput}
        updatedAt={updatedAt}
        formatHz={formatHz}
        onClose={() => setSignalOpen(false)}
        />
      ) : null}

      {!showGate ? (
        <MusicBrainzMatchModal
        open={Boolean(matchTarget)}
        kind="track"
        targetLabel={matchLabel}
        defaults={matchDefaults}
        trackPath={matchTarget?.path ?? null}
        onClose={() => setMatchTarget(null)}
        />
      ) : null}

      {!showGate ? (
        <TrackMetadataModal
        open={Boolean(editTarget)}
        trackId={editTarget?.trackId ?? null}
        trackPath={editTarget?.path ?? null}
        targetLabel={editLabel}
        defaults={editDefaults}
        onClose={() => setEditTarget(null)}
        onSaved={() => {
          if (albumViewId !== null) {
            loadAlbumTracks(albumViewId);
          }
          loadAlbums();
        }}
        />
      ) : null}

      {!showGate ? (
        <AlbumMetadataDialog
        open={Boolean(albumEditTarget)}
        albumId={albumEditTarget?.albumId ?? null}
        targetLabel={albumEditLabel}
        artist={albumEditTarget?.artist ?? ""}
        defaults={albumEditDefaults}
        onBeforeUpdate={async () => {
          if (!albumEditTarget?.albumId) return;
          if (nowPlayingAlbumId !== albumEditTarget.albumId) return;
          if (!isPlaying) return;
          await handlePause();
        }}
        onClose={() => setAlbumEditTarget(null)}
        onUpdated={(updatedAlbumId) => {
          if (albumViewId !== null) {
            setAlbumViewId(updatedAlbumId);
            loadAlbumTracks(updatedAlbumId);
          }
          loadAlbums();
        }}
        />
      ) : null}

      {!showGate ? (
        <QueueModal
        open={queueOpen}
        items={queue}
        onClose={() => setQueueOpen(false)}
        formatMs={formatMs}
        placeholder={albumPlaceholder}
        canPlay={Boolean(activeOutputId)}
        onPlayFrom={handleQueuePlayFrom}
        />
      ) : null}

      {!showGate ? <audio ref={audioRef} preload="auto" style={{ display: "none" }} /> : null}

    </div>
  );
}
