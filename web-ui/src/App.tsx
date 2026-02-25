import { useEffect, useMemo, useState, useCallback, useRef, SetStateAction} from "react";
import { toast, ToastContainer } from "react-toastify";
import {
  Bell,
  ChevronLeft,
  ChevronRight,
  Grid3x3,
  Library,
  List,
  PanelLeftClose,
  PanelLeftOpen,
  Radio,
  Search,
  Settings,
  Trash2
} from "lucide-react";
import {
  apiUrl,
  fetchJson,
  getDefaultApiBase,
  getEffectiveApiBase,
  getStoredApiBase,
  postJson,
  safeMediaUrl,
  setStoredApiBase
} from "./api";
import {
  AlbumListResponse,
  AlbumProfileResponse,
  AlbumSummary,
  LogEvent,
  MetadataEvent,
  OutputInfo,
  OutputSettings,
  OutputSettingsResponse,
  ProviderOutputs,
  SessionCreateResponse,
  SessionDetailResponse,
  SessionLocksResponse,
  SessionsListResponse,
  SessionSummary,
  StatusResponse,
  QueueItem,
  TrackResolveResponse,
  TrackListResponse,
  TrackSummary
} from "./types";
import AlbumDetailView from "./components/AlbumDetailView";
import AlbumMetadataDialog from "./components/AlbumMetadataDialog";
import AlbumsView from "./components/AlbumsView";
import CatalogMetadataDialog from "./components/CatalogMetadataDialog";
import AlbumNotesModal from "./components/AlbumNotesModal";
import MusicBrainzMatchModal from "./components/MusicBrainzMatchModal";
import Modal from "./components/Modal";
import TrackMetadataModal from "./components/TrackMetadataModal";
import TrackAnalysisModal from "./components/TrackAnalysisModal";
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
import { usePlaybackActions } from "./hooks/usePlaybackActions";
import { useQueueActions } from "./hooks/useQueueActions";

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

type LocalPlaybackCommand = {
  url: string;
  path: string;
  track_id?: number | null;
};

const MAX_METADATA_EVENTS = 200;
const MAX_LOG_EVENTS = 300;
const TRACK_MENU_GAP_PX = 4;
const TRACK_MENU_MARGIN_PX = 8;
const TRACK_MENU_MIN_WIDTH_PX = 220;
const TRACK_MENU_ESTIMATED_HEIGHT_PX = 320;
const WEB_SESSION_CLIENT_ID_KEY = "audioHub.webSessionClientId";
const WEB_SESSION_ID_KEY = "audioHub.webSessionId";
const NAV_COLLAPSED_KEY = "audioHub.navCollapsed";
const WEB_DEFAULT_SESSION_NAME = "Default";
const LOCAL_PLAYBACK_SNAPSHOT_KEY_PREFIX = "audioHub.localPlaybackSnapshot:";

type LocalPlaybackSnapshot = {
  path: string;
  paused: boolean;
  elapsed_ms: number | null;
  duration_ms: number | null;
  title: string | null;
  artist: string | null;
  album: string | null;
  saved_at_ms: number;
};

function localPlaybackSnapshotKey(sessionId: string): string {
  return `${LOCAL_PLAYBACK_SNAPSHOT_KEY_PREFIX}${sessionId}`;
}

function loadLocalPlaybackSnapshot(sessionId: string): LocalPlaybackSnapshot | null {
  try {
    const raw = localStorage.getItem(localPlaybackSnapshotKey(sessionId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as LocalPlaybackSnapshot;
    if (!parsed?.path) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveLocalPlaybackSnapshot(sessionId: string, snapshot: LocalPlaybackSnapshot): void {
  try {
    localStorage.setItem(localPlaybackSnapshotKey(sessionId), JSON.stringify(snapshot));
  } catch {
    // ignore storage failures
  }
}

function isDefaultSessionName(name: string | null | undefined): boolean {
  return (name ?? "").trim().toLowerCase() === WEB_DEFAULT_SESSION_NAME.toLowerCase();
}

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

function fileNameFromPath(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] || path;
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

function getOrCreateWebSessionClientId(): string {
  try {
    const existing = localStorage.getItem(WEB_SESSION_CLIENT_ID_KEY);
    if (existing) return existing;
    const generated =
      typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
    localStorage.setItem(WEB_SESSION_CLIENT_ID_KEY, generated);
    return generated;
  } catch {
    return `web-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  }
}

export default function App() {
  const [outputs, setOutputs] = useState<OutputInfo[]>([]);
  const [activeOutputId, setActiveOutputId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(() => {
    try {
      return localStorage.getItem(WEB_SESSION_ID_KEY);
    } catch {
      return null;
    }
  });
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [sessionOutputLocks, setSessionOutputLocks] = useState<
    SessionLocksResponse["output_locks"]
  >([]);
  const [sessionBridgeLocks, setSessionBridgeLocks] = useState<
    SessionLocksResponse["bridge_locks"]
  >([]);
  const [createSessionOpen, setCreateSessionOpen] = useState<boolean>(false);
  const [newSessionName, setNewSessionName] = useState<string>("");
  const [newSessionNeverExpires, setNewSessionNeverExpires] = useState<boolean>(false);
  const [createSessionBusy, setCreateSessionBusy] = useState<boolean>(false);
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [rescanBusy, setRescanBusy] = useState<boolean>(false);
  const [trackMenuPath, setTrackMenuPath] = useState<string | null>(null);
  const [trackMenuPosition, setTrackMenuPosition] = useState<{
    top: number;
    right: number;
    up: boolean;
  } | null>(null);
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
      return localStorage.getItem(NAV_COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [settingsSection, setSettingsSection] = useState<"metadata" | "logs" | "connection" | "outputs">("metadata");
  const [outputsSettings, setOutputsSettings] = useState<OutputSettings | null>(null);
  const [outputsProviders, setOutputsProviders] = useState<ProviderOutputs[]>([]);
  const [outputsLoading, setOutputsLoading] = useState<boolean>(false);
  const [outputsError, setOutputsError] = useState<string | null>(null);
  const [outputsLastRefresh, setOutputsLastRefresh] = useState<Record<string, string>>({});
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
  const [albumProfile, setAlbumProfile] = useState<AlbumProfileResponse | null>(null);
  const [catalogLoading, setCatalogLoading] = useState<boolean>(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
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
  const localPathRef = useRef<string | null>(null);
  const notificationIdRef = useRef(0);
  const toastLastRef = useRef<{ message: string; level: ToastLevel; at: number } | null>(null);
  const activeSessionIdRef = useRef<string | null>(sessionId);
  const isLocalSessionRef = useRef<boolean>(false);

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
  const currentSession = useMemo(
    () => sessions.find((session) => session.id === sessionId) ?? null,
    [sessions, sessionId]
  );
  const isLocalSession = currentSession?.mode === "local";
  const canTogglePlayback = Boolean(
    sessionId && status?.now_playing && (isLocalSession || activeOutputId)
  );
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
    settingsSection?: "metadata" | "logs" | "connection" | "outputs";
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
  const playButtonTitle = !sessionId
    ? "Creating session..."
    : !activeOutputId && !isLocalSession
    ? (isLocalSession
      ? "Local session is ready."
      : "Select an output to control playback.")
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
      const originalYear = album.original_year ? String(album.original_year) : "";
      const editionYear = album.edition_year ? String(album.edition_year) : "";
      const editionLabel = album.edition_label?.toLowerCase() ?? "";
      return (
        title.includes(query) ||
        artist.includes(query) ||
        year.includes(query) ||
        originalYear.includes(query) ||
        editionYear.includes(query) ||
        editionLabel.includes(query)
      );
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

  useEffect(() => {
    try {
      localStorage.setItem(NAV_COLLAPSED_KEY, navCollapsed ? "1" : "0");
    } catch {
      // ignore storage failures
    }
  }, [navCollapsed]);

  useEffect(() => {
    activeSessionIdRef.current = sessionId;
    isLocalSessionRef.current = Boolean(isLocalSession);
  }, [isLocalSession, sessionId]);

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

  const refreshSessions = useCallback(async () => {
    const clientId = getOrCreateWebSessionClientId();
    const response = await fetchJson<SessionsListResponse>(
      `/sessions?client_id=${encodeURIComponent(clientId)}`
    );
    setSessions(response.sessions ?? []);
  }, []);

  const refreshSessionLocks = useCallback(async () => {
    const response = await fetchJson<SessionLocksResponse>("/sessions/locks");
    setSessionOutputLocks(response.output_locks ?? []);
    setSessionBridgeLocks(response.bridge_locks ?? []);
  }, []);

  const refreshSessionDetail = useCallback(async (id: string) => {
    const clientId = getOrCreateWebSessionClientId();
    const detail = await fetchJson<SessionDetailResponse>(
      `/sessions/${encodeURIComponent(id)}?client_id=${encodeURIComponent(clientId)}`
    );
    setActiveOutputId(detail.active_output_id ?? null);
  }, []);

  const ensureSession = useCallback(async () => {
    const clientId = getOrCreateWebSessionClientId();
    const defaultSession = await postJson<SessionCreateResponse>("/sessions", {
      name: WEB_DEFAULT_SESSION_NAME,
      mode: "remote",
      client_id: `${clientId}:default`,
      app_version: __APP_VERSION__,
      owner: "web-ui",
      lease_ttl_sec: 0
    });
    await postJson<SessionCreateResponse>("/sessions", {
      name: "Local",
      mode: "local",
      client_id: clientId,
      app_version: __APP_VERSION__,
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
        return localStorage.getItem(WEB_SESSION_ID_KEY);
      } catch {
        return null;
      }
    })();
    const nextSessionId =
      (stored && nextSessions.some((session) => session.id === stored) ? stored : null) ??
      nextSessions.find((session) => isDefaultSessionName(session.name))?.id ??
      defaultSession.session_id;
    setSessionId(nextSessionId);
    try {
      localStorage.setItem(WEB_SESSION_ID_KEY, nextSessionId);
    } catch {
      // ignore storage failures
    }
    await refreshSessionDetail(nextSessionId);
  }, [refreshSessionDetail, refreshSessionLocks]);

  const handleSessionChange = useCallback(
    async (nextSessionId: string) => {
      setSessionId(nextSessionId);
      setActiveOutputId(null);
      setStatus(null);
      setQueue([]);
      localPathRef.current = null;
      try {
        localStorage.setItem(WEB_SESSION_ID_KEY, nextSessionId);
      } catch {
        // ignore storage failures
      }
      try {
        await refreshSessionDetail(nextSessionId);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [refreshSessionDetail, reportError]
  );

  const createNamedSession = useCallback(async (name: string, neverExpires = false) => {
    try {
      const response = await postJson<SessionCreateResponse>("/sessions", {
        name,
        mode: "remote",
        client_id:
          typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
            ? `${getOrCreateWebSessionClientId()}:${crypto.randomUUID()}`
            : `${getOrCreateWebSessionClientId()}-${Date.now()}`,
        app_version: __APP_VERSION__,
        owner: "web-ui",
        ...(neverExpires ? { lease_ttl_sec: 0 } : {})
      });
      await Promise.all([refreshSessions(), refreshSessionLocks()]);
      await handleSessionChange(response.session_id);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [refreshSessionLocks, refreshSessions, handleSessionChange, reportError]);

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
      const clientId = getOrCreateWebSessionClientId();
      const sessionsResponse = await fetchJson<SessionsListResponse>(
        `/sessions?client_id=${encodeURIComponent(clientId)}`
      );
      const nextSessions = sessionsResponse.sessions ?? [];
      setSessions(nextSessions);
      await refreshSessionLocks();
      const defaultSession =
        nextSessions.find((item) => isDefaultSessionName(item.name)) ?? nextSessions[0] ?? null;
      if (defaultSession) {
        await handleSessionChange(defaultSession.id);
      } else {
        setSessionId(null);
        setActiveOutputId(null);
        setStatus(null);
      }
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [sessionId, sessions, refreshSessionLocks, handleSessionChange, reportError]);

  useEffect(() => {
    if (!serverConnected) return;
    ensureSession().catch((err) => {
      reportError((err as Error).message);
    });
  }, [serverConnected, apiBaseOverride, ensureSession, reportError]);

  useEffect(() => {
    if (!serverConnected || !sessionId) return;
    refreshSessionDetail(sessionId).catch(() => {
      // Session may have expired or been removed.
    });
  }, [serverConnected, sessionId, refreshSessionDetail]);

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
            `/sessions?client_id=${encodeURIComponent(getOrCreateWebSessionClientId())}`
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
  }, [serverConnected]);

  const streamKey = useMemo(
    () => `${apiBaseOverride}:${serverConnected ? "up" : "down"}:${sessionId ?? "none"}`,
    [apiBaseOverride, serverConnected, sessionId]
  );

  useOutputsStream({
    enabled: serverConnected,
    sourceKey: streamKey,
    onEvent: (data) => {
      setOutputs(data.outputs);
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
    handlePause: handlePauseRemote,
    handleSelectOutput,
    handlePlay: handlePlayRemote,
    handlePlayAlbumTrack: handlePlayAlbumTrackRemote,
    handlePlayAlbumById: handlePlayAlbumByIdRemote
  } = usePlaybackActions({
    sessionId,
    activeOutputId,
    rescanBusy,
    setError: reportError,
    setActiveOutputId,
    setRescanBusy
  });
  const {
    handleNext: handleNextRemote,
    handlePrevious: handlePreviousRemote,
    handleQueue,
    handlePlayNext,
    handleQueueClear,
    handleQueuePlayFrom: handleQueuePlayFromRemote
  } = useQueueActions({ sessionId, setError: reportError });

  const handlePause = useCallback(async () => {
    if (isLocalSession) {
      const audio = audioRef.current;
      if (!audio || !localPathRef.current) return;
      if (audio.paused) {
        await audio.play().catch(() => {});
      } else {
        audio.pause();
      }
      updateLocalStatusFromAudio();
      return;
    }
    await handlePauseRemote();
  }, [handlePauseRemote, isLocalSession]);

  const handlePlay = useCallback(
    async (path: string) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayRemote(path);
          return;
        }
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, { paths: [path] });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [handlePlayRemote, isLocalSession, reportError, sessionId]
  );

  const handleNext = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handleNextRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/next");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [handleNextRemote, isLocalSession, reportError]);

  const handlePrevious = useCallback(async () => {
    try {
      if (!isLocalSession) {
        await handlePreviousRemote();
        return;
      }
      const payload = await requestLocalCommand("/queue/previous");
      await applyLocalPlayback(payload);
    } catch (err) {
      reportError((err as Error).message);
    }
  }, [handlePreviousRemote, isLocalSession, reportError]);

  const handleQueuePlayFrom = useCallback(
    async (payload: { trackId?: number; path?: string }) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handleQueuePlayFromRemote(payload);
          return;
        }
        const endpoint = `/sessions/${encodeURIComponent(sessionId)}/queue/play_from`;
        const body = payload.trackId ? { track_id: payload.trackId } : { path: payload.path };
        const command = await postJson<LocalPlaybackCommand>(endpoint, body as any);
        await applyLocalPlayback(command);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [handleQueuePlayFromRemote, isLocalSession, reportError, sessionId]
  );

  const handlePlayAlbumTrack = useCallback(
    async (track: TrackSummary) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumTrackRemote(track);
          return;
        }
        if (!track.path) return;
        await postJson(`/sessions/${encodeURIComponent(sessionId)}/queue/next/add`, {
          paths: [track.path]
        });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      handlePlayAlbumTrackRemote,
      isLocalSession,
      reportError,
      sessionId
    ]
  );

  const handlePlayAlbumById = useCallback(
    async (albumId: number) => {
      try {
        if (!isLocalSession || !sessionId) {
          await handlePlayAlbumByIdRemote(albumId);
          return;
        }
        const tracks = await fetchJson<TrackListResponse>(`/tracks?album_id=${albumId}&limit=500`);
        const paths = (tracks.items ?? [])
          .map((track) => track.path)
          .filter((path): path is string => Boolean(path));
        if (!paths.length) {
          throw new Error("Album has no playable tracks.");
        }
        const base = `/sessions/${encodeURIComponent(sessionId)}/queue`;
        await postJson(`${base}/clear`, {
          clear_queue: true,
          clear_history: false
        });
        await postJson(base, { paths });
        const payload = await requestLocalCommand("/queue/next");
        await applyLocalPlayback(payload);
      } catch (err) {
        reportError((err as Error).message);
      }
    },
    [
      handlePlayAlbumByIdRemote,
      isLocalSession,
      reportError,
      sessionId
    ]
  );

  const handleSelectOutputForSession = useCallback(
    async (id: string) => {
      if (isLocalSession) return;
      await handleSelectOutput(id, false);
      if (!sessionId) return;
      try {
        await Promise.all([refreshSessions(), refreshSessionLocks(), refreshSessionDetail(sessionId)]);
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

  useMetadataStream({
    enabled: settingsOpen && serverConnected && settingsSection === "metadata",
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
    enabled: settingsOpen && serverConnected && settingsSection === "logs",
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

  const fetchOutputSettings = useCallback(async () => {
    setOutputsLoading(true);
    try {
      const data = await fetchJson<OutputSettingsResponse>("/outputs/settings");
      setOutputsSettings(data.settings);
      setOutputsProviders(data.providers);
      setOutputsError(null);
    } catch (error) {
      setOutputsError(error instanceof Error ? error.message : "Failed to load outputs");
    } finally {
      setOutputsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!settingsOpen || settingsSection !== "outputs" || !serverConnected) return;
    fetchOutputSettings();
  }, [settingsOpen, settingsSection, serverConnected, fetchOutputSettings]);

  const updateOutputSettings = useCallback(async (next: OutputSettings) => {
    const data = await fetchJson<OutputSettings>("/outputs/settings", {
      method: "POST",
      body: JSON.stringify(next)
    });
    setOutputsSettings(data);
  }, []);

  const handleToggleOutputSetting = useCallback(async (outputId: string, enabled: boolean) => {
    if (!outputsSettings) return;
    const disabled = new Set(outputsSettings.disabled);
    if (enabled) {
      disabled.delete(outputId);
    } else {
      disabled.add(outputId);
    }
    const next: OutputSettings = {
      ...outputsSettings,
      disabled: Array.from(disabled)
    };
    setOutputsSettings(next);
    try {
      await updateOutputSettings(next);
    } catch (error) {
      setOutputsSettings(outputsSettings);
      setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
    }
  }, [outputsSettings, updateOutputSettings]);

  const handleRenameOutputSetting = useCallback(async (outputId: string, name: string) => {
    if (!outputsSettings) return;
    const renames = { ...outputsSettings.renames };
    if (name) {
      renames[outputId] = name;
    } else {
      delete renames[outputId];
    }
    const next: OutputSettings = {
      ...outputsSettings,
      renames
    };
    setOutputsSettings(next);
    try {
      await updateOutputSettings(next);
    } catch (error) {
      setOutputsSettings(outputsSettings);
      setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
    }
  }, [outputsSettings, updateOutputSettings]);

  const handleToggleExclusiveSetting = useCallback(async (outputId: string, enabled: boolean) => {
    if (!outputsSettings) return;
    const exclusive = new Set(outputsSettings.exclusive);
    if (enabled) {
      exclusive.add(outputId);
    } else {
      exclusive.delete(outputId);
    }
    const next: OutputSettings = {
      ...outputsSettings,
      exclusive: Array.from(exclusive)
    };
    setOutputsSettings(next);
    try {
      await updateOutputSettings(next);
    } catch (error) {
      setOutputsSettings(outputsSettings);
      setOutputsError(error instanceof Error ? error.message : "Failed to update outputs");
    }
  }, [outputsSettings, updateOutputSettings]);

  const handleRefreshProvider = useCallback(async (providerId: string) => {
    try {
      await postJson(`/providers/${encodeURIComponent(providerId)}/refresh`);
      const now = new Date();
      setOutputsLastRefresh((prev) => ({
        ...prev,
        [providerId]: now.toLocaleTimeString()
      }));
      fetchOutputSettings();
    } catch (error) {
      setOutputsError(error instanceof Error ? error.message : "Failed to refresh provider");
    }
  }, [fetchOutputSettings]);

  useStatusStream({
    enabled: serverConnected && !isLocalSession && Boolean(sessionId && activeOutputId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (data: SetStateAction<StatusResponse | null>) => {
      if (isLocalSessionRef.current) {
        return;
      }
      if (!sessionId || activeSessionIdRef.current !== sessionId) {
        return;
      }
      setStatus(data);
      setUpdatedAt(new Date());
      markServerConnected();
    },
    onError: () => {
      if (!activeOutputId) {
        return;
      }
      const message = connectionError(
        "Live status disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/status/stream`
          : "/sessions/{id}/status/stream"
      );
      reportError(message, "warn");
    }
  });

  const updateLocalStatusFromAudio = useCallback(
    (base?: Partial<StatusResponse>) => {
      if (!isLocalSession) return;
      const audio = audioRef.current;
      if (!audio) return;
      const hasTrack = Boolean(localPathRef.current);
      setStatus((prev) => {
        const next: StatusResponse = {
          ...(prev ?? {}),
          ...base,
          now_playing: hasTrack ? localPathRef.current : null,
          paused: hasTrack ? audio.paused : true,
          elapsed_ms:
            hasTrack && Number.isFinite(audio.currentTime)
              ? Math.floor(audio.currentTime * 1000)
              : null,
          duration_ms:
            hasTrack && Number.isFinite(audio.duration) ? Math.floor(audio.duration * 1000) : null
        };
        if (!hasTrack) {
          next.title = null;
          next.artist = null;
          next.album = null;
          next.output_sample_rate = null;
          next.channels = null;
        }
        return next;
      });
      setUpdatedAt(new Date());
    },
    [isLocalSession]
  );

  const applyLocalPlayback = useCallback(
    async (payload: LocalPlaybackCommand | null) => {
      const audio = audioRef.current;
      if (!audio) return;
      if (!payload?.url || !payload.path) {
        audio.pause();
        audio.removeAttribute("src");
        audio.load();
        localPathRef.current = null;
        updateLocalStatusFromAudio();
        return;
      }
      const safeUrl = safeMediaUrl(payload.url);
      if (!safeUrl) {
        reportError("Rejected local playback URL.");
        return;
      }
      localPathRef.current = payload.path;
      const queueTrack = queue.find(
        (item) => item.kind === "track" && item.path === payload.path
      );
      audio.src = safeUrl;
      audio.load();
      await audio.play().catch(() => {});
      updateLocalStatusFromAudio({
        title:
          queueTrack?.kind === "track"
            ? (queueTrack.title ?? queueTrack.file_name)
            : fileNameFromPath(payload.path),
        artist: queueTrack?.kind === "track" ? (queueTrack.artist ?? null) : null,
        album: queueTrack?.kind === "track" ? (queueTrack.album ?? null) : null
      });
    },
    [queue, reportError, updateLocalStatusFromAudio]
  );

  const requestLocalCommand = useCallback(
    async (
      endpoint: string,
      body?: Record<string, string | number | boolean | null | undefined>
    ): Promise<LocalPlaybackCommand | null> => {
      if (!sessionId) return null;
      const response = await postJson<LocalPlaybackCommand | null>(
        `/sessions/${encodeURIComponent(sessionId)}${endpoint}`,
        body as any
      );
      if (!response || !response.url || !response.path) {
        return null;
      }
      return response;
    },
    [sessionId]
  );

  useEffect(() => {
    if (!isLocalSession) return;
    const audio = audioRef.current;
    if (!audio) return;
    const onTimeUpdate = () => updateLocalStatusFromAudio();
    const onPause = () => updateLocalStatusFromAudio();
    const onPlay = () => updateLocalStatusFromAudio();
    const onDurationChange = () => updateLocalStatusFromAudio();
    const onEnded = () => {
      requestLocalCommand("/queue/next")
        .then((payload) => applyLocalPlayback(payload))
        .catch((err) => reportError((err as Error).message));
    };
    audio.addEventListener("timeupdate", onTimeUpdate);
    audio.addEventListener("pause", onPause);
    audio.addEventListener("play", onPlay);
    audio.addEventListener("durationchange", onDurationChange);
    audio.addEventListener("ended", onEnded);
    return () => {
      audio.removeEventListener("timeupdate", onTimeUpdate);
      audio.removeEventListener("pause", onPause);
      audio.removeEventListener("play", onPlay);
      audio.removeEventListener("durationchange", onDurationChange);
      audio.removeEventListener("ended", onEnded);
    };
  }, [applyLocalPlayback, isLocalSession, reportError, requestLocalCommand, updateLocalStatusFromAudio]);
  useEffect(() => {
    if (!sessionId || (!activeOutputId && !isLocalSession)) {
      setStatus(null);
    }
  }, [sessionId, activeOutputId, isLocalSession]);

  useEffect(() => {
    if (!isLocalSession || !sessionId) return;
    const currentPath = status?.now_playing ?? null;
    if (!currentPath) {
      return;
    }
    saveLocalPlaybackSnapshot(sessionId, {
      path: currentPath,
      paused: Boolean(status?.paused ?? true),
      elapsed_ms: status?.elapsed_ms ?? null,
      duration_ms: status?.duration_ms ?? null,
      title: status?.title ?? null,
      artist: status?.artist ?? null,
      album: status?.album ?? null,
      saved_at_ms: Date.now()
    });
  }, [
    isLocalSession,
    sessionId,
    status?.album,
    status?.artist,
    status?.duration_ms,
    status?.elapsed_ms,
    status?.now_playing,
    status?.paused,
    status?.title
  ]);


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

  useEffect(() => {
    if (!isLocalSession) return;
    const path = status?.now_playing ?? null;
    if (!path) return;
    const queueTrack = queue.find(
      (item) => item.kind === "track" && item.path === path
    );
    if (!queueTrack || queueTrack.kind !== "track") return;

    const nextTitle = queueTrack.title ?? queueTrack.file_name ?? fileNameFromPath(path);
    const nextArtist = queueTrack.artist ?? null;
    const nextAlbum = queueTrack.album ?? null;
    if (
      status?.title === nextTitle &&
      (status?.artist ?? null) === nextArtist &&
      (status?.album ?? null) === nextAlbum
    ) {
      return;
    }
    setStatus((prev) =>
      prev
        ? {
            ...prev,
            title: nextTitle,
            artist: nextArtist,
            album: nextAlbum
          }
        : prev
    );
  }, [isLocalSession, queue, status?.album, status?.artist, status?.now_playing, status?.title]);

  useEffect(() => {
    if (!isLocalSession || !sessionId) return;
    const currentQueueItem = queue.find(
      (item) => item.kind === "track" && item.now_playing
    );
    if (!currentQueueItem || currentQueueItem.kind !== "track") {
      return;
    }
    if (status?.now_playing === currentQueueItem.path) {
      return;
    }

    const snapshot = loadLocalPlaybackSnapshot(sessionId);
    const path = currentQueueItem.path;
    const title = currentQueueItem.title ?? currentQueueItem.file_name ?? fileNameFromPath(path);
    const artist = currentQueueItem.artist ?? null;
    const album = currentQueueItem.album ?? null;
    const elapsedMs =
      snapshot?.path === path ? (snapshot.elapsed_ms ?? null) : null;
    const durationMs =
      snapshot?.path === path
        ? (snapshot.duration_ms ?? currentQueueItem.duration_ms ?? null)
        : (currentQueueItem.duration_ms ?? null);

    localPathRef.current = path;
    setStatus((prev) => ({
      ...(prev ?? {}),
      now_playing: path,
      paused: true,
      elapsed_ms: elapsedMs,
      duration_ms: durationMs,
      title,
      artist,
      album
    }));
    setUpdatedAt(new Date());
  }, [isLocalSession, queue, sessionId, status?.now_playing]);

  useEffect(() => {
    if (!isLocalSession) return;
    const hasLocalNowPlaying = queue.some((item) => item.kind === "track" && item.now_playing);
    if (hasLocalNowPlaying) return;
    if (!status?.now_playing) return;
    setStatus(null);
    localPathRef.current = null;
  }, [isLocalSession, queue, status?.now_playing]);

  useQueueStream({
    enabled: serverConnected && Boolean(sessionId),
    sourceKey: streamKey,
    sessionId,
    onEvent: (items) => {
      setQueue(items ?? []);
      markServerConnected();
    },
    onError: () => {
      const message = connectionError(
        "Live queue disconnected",
        sessionId
          ? `/sessions/${encodeURIComponent(sessionId)}/queue/stream`
          : "/sessions/{id}/queue/stream"
      );
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

  const loadCatalogProfiles = useCallback(async (albumId: number | null) => {
    if (albumId === null) {
      setAlbumProfile(null);
      setCatalogError(null);
      return;
    }
    setCatalogError(null);
    setCatalogLoading(true);
    try {
      const albumPromise = fetchJson<AlbumProfileResponse>(
        `/albums/profile?album_id=${albumId}&lang=en-US`
      );
      const [albumResult] = await Promise.allSettled([albumPromise]);
      if (albumResult.status === "fulfilled") {
        setAlbumProfile(albumResult.value);
      } else {
        setCatalogError(albumResult.reason instanceof Error ? albumResult.reason.message : String(albumResult.reason));
      }
    } catch (err) {
      setCatalogError((err as Error).message);
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!serverConnected) return;
    loadAlbumTracks(albumViewId);
  }, [albumViewId, loadAlbumTracks, serverConnected]);

  useEffect(() => {
    setAlbumNotesOpen(false);
  }, [albumViewId]);

  useEffect(() => {
    if (albumViewId === null) return;
    const main = document.querySelector<HTMLElement>(".main");
    if (main) {
      main.scrollTo({ top: 0, behavior: "smooth" });
    } else {
      window.scrollTo({ top: 0, behavior: "smooth" });
    }
  }, [albumViewId]);

  useEffect(() => {
    if (!serverConnected) return;
    loadCatalogProfiles(albumViewId);
  }, [albumViewId, loadCatalogProfiles, serverConnected]);

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
        handlePrevious();
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
    handlePrevious,
    nowPlayingCover,
    status?.album,
    status?.artist,
    status?.title
  ]);

  async function handlePrimaryAction() {
    if (status?.now_playing) {
      if (isLocalSession && status.paused && sessionId) {
        const audio = audioRef.current;
        const hasSource = Boolean(audio?.src);
        if (!hasSource) {
          try {
            const payload = await postJson<LocalPlaybackCommand>(
              `/sessions/${encodeURIComponent(sessionId)}/queue/play_from`,
              { path: status.now_playing }
            );
            await applyLocalPlayback(payload);
            const seekMs = status.elapsed_ms ?? null;
            if (audioRef.current && seekMs && seekMs > 0) {
              const resumeAt = seekMs / 1000;
              const player = audioRef.current;
              const applySeek = () => {
                player.currentTime = resumeAt;
              };
              if (Number.isFinite(player.duration) && player.duration > 0) {
                applySeek();
              } else {
                const onLoaded = () => {
                  player.removeEventListener("loadedmetadata", onLoaded);
                  applySeek();
                };
                player.addEventListener("loadedmetadata", onLoaded);
              }
            }
            return;
          } catch (err) {
            reportError((err as Error).message);
            return;
          }
        }
      }
      await handlePause();
      return;
    }
  }

  const showGate = !serverConnected;
  const queueHasNext = Boolean(sessionId && (activeOutputId || isLocalSession)) && queue.some((item) =>
    item.kind === "track" ? !item.now_playing : true
  );
  const canGoPrevious = isLocalSession
    ? queue.some((item) => item.kind === "track" && Boolean(item.played))
    : Boolean(status?.has_previous);
  return (
    <div className={`app ${settingsOpen ? "settings-mode" : ""} ${showGate ? "has-gate" : ""} ${queueOpen ? "queue-open" : ""}`}>
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
      <div className={`layout ${navCollapsed ? "nav-collapsed" : ""}`}>
        <aside className="side-nav">
          <div className="nav-brand">
            <div className="nav-brand-text">
              <div className="nav-title">Audio Hub</div>
              <div className="nav-subtitle">Lossless control with a live signal view.</div>
            </div>
            <button
              className="icon-btn nav-collapse"
              onClick={() => setNavCollapsed((prev) => !prev)}
              aria-label={navCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              title={navCollapsed ? "Expand sidebar" : "Collapse sidebar"}
              type="button"
            >
              {navCollapsed ? (
                <PanelLeftOpen className="icon" aria-hidden="true" />
              ) : (
                <PanelLeftClose className="icon" aria-hidden="true" />
              )}
            </button>
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
                <div className="header-session">
                  <select
                    className="header-session-select"
                    value={sessionId ?? ""}
                    onChange={(event) => handleSessionChange(event.target.value)}
                    aria-label="Playback session"
                    title="Playback session"
                    disabled={!serverConnected || sessions.length === 0}
                  >
                    {sessions.length === 0 ? (
                      <option value="">No session</option>
                    ) : null}
                    {sessions.map((session) => (
                      <option key={session.id} value={session.id}>
                        {session.name}
                      </option>
                    ))}
                  </select>
                  <button
                    className="icon-btn"
                    type="button"
                    onClick={handleCreateSession}
                    title="Create new session"
                    aria-label="Create new session"
                    disabled={!serverConnected}
                  >
                    <Radio className="icon" aria-hidden="true" />
                  </button>
                  <button
                    className="icon-btn"
                    type="button"
                    onClick={() => {
                      void handleDeleteSession();
                    }}
                    title="Delete selected session"
                    aria-label="Delete selected session"
                    disabled={
                      !serverConnected ||
                      !sessionId ||
                      sessions.find((item) => item.id === sessionId)?.mode === "local" ||
                      isDefaultSessionName(
                        sessions.find((item) => item.id === sessionId)?.name
                      )
                    }
                  >
                    <Trash2 className="icon" aria-hidden="true" />
                  </button>
                </div>
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
                canPlay={Boolean(sessionId && (activeOutputId || isLocalSession))}
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
              canPlay={Boolean(sessionId && (activeOutputId || isLocalSession)) && albumTracks.length > 0}
              activeAlbumId={activeAlbumId}
              isPlaying={isPlaying}
              isPaused={isPaused}
              onPause={handlePause}
              formatMs={formatMs}
              nowPlayingPath={status?.now_playing ?? null}
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
              onAnalyzeTrack={(track) => {
                runTrackMenuAction(() => {
                  setAnalysisTarget({
                    trackId: track.id,
                    title: track.title ?? track.file_name,
                    artist: track.artist ?? null
                  });
                }, track.path);
              }}
              onEditAlbumMetadata={openAlbumEditor}
              onEditCatalogMetadata={() => setCatalogOpen(true)}
              onReadAlbumNotes={() => setAlbumNotesOpen(true)}
              albumProfile={albumProfile}
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
            outputsSettings={outputsSettings}
            outputsProviders={outputsProviders}
            outputsLoading={outputsLoading}
            outputsError={outputsError}
            outputsLastRefresh={outputsLastRefresh}
            onRefreshProvider={handleRefreshProvider}
            onToggleOutput={handleToggleOutputSetting}
            onRenameOutput={handleRenameOutputSetting}
            onToggleExclusive={handleToggleExclusiveSetting}
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
        <div
          className="side-panel-backdrop notifications-backdrop"
          onClick={() => setNotificationsOpen(false)}
        >
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
          showSignalAction={!isLocalSession}
          showSignalPath={isPlaying}
          canTogglePlayback={canTogglePlayback}
          canGoPrevious={canGoPrevious}
          playButtonTitle={playButtonTitle}
          queueHasItems={queueHasNext}
          queueOpen={queueOpen}
          showOutputAction={!isLocalSession}
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
          onPrevious={handlePrevious}
          onNext={handleNext}
          onSignalOpen={() => setSignalOpen(true)}
          onQueueOpen={() => setQueueOpen((value) => !value)}
          onSelectOutput={() => {
            if (!isLocalSession) {
              setOutputsOpen(true);
            }
          }}
        />
      ) : null}

      {!showGate ? (
        <Modal
        open={createSessionOpen}
        title="Create session"
        onClose={() => {
          if (!createSessionBusy) {
            setCreateSessionOpen(false);
          }
        }}
        >
          <div className="modal-body">
            <label className="mb-match-field">
              <span>Session name</span>
              <input
                className="mb-match-input"
                type="text"
                value={newSessionName}
                onChange={(event) => setNewSessionName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    if (!createSessionBusy) {
                      void submitCreateSession();
                    }
                  }
                }}
                autoFocus
                maxLength={80}
                placeholder="My session"
              />
            </label>
            <label className="modal-checkbox">
              <input
                type="checkbox"
                checked={newSessionNeverExpires}
                onChange={(event) => setNewSessionNeverExpires(event.target.checked)}
                disabled={createSessionBusy}
              />
              Never expires
            </label>
            <div className="modal-actions">
              <button
                className="btn ghost"
                type="button"
                onClick={() => setCreateSessionOpen(false)}
                disabled={createSessionBusy}
              >
                Cancel
              </button>
              <button
                className="btn"
                type="button"
                onClick={() => {
                  void submitCreateSession();
                }}
                disabled={createSessionBusy || newSessionName.trim().length === 0}
              >
                {createSessionBusy ? "Creating..." : "Create"}
              </button>
            </div>
          </div>
        </Modal>
      ) : null}

      {!showGate && !isLocalSession ? (
        <OutputsModal
        open={outputsOpen}
        outputs={outputs}
        sessions={sessions}
        outputLocks={sessionOutputLocks}
        bridgeLocks={sessionBridgeLocks}
        currentSessionId={sessionId}
        activeOutputId={activeOutputId}
        onClose={() => setOutputsOpen(false)}
        onSelectOutput={handleSelectOutputForSession}
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
        <AlbumNotesModal
        open={albumNotesOpen}
        title={selectedAlbum?.title ?? ""}
        artist={selectedAlbum?.artist ?? ""}
        notes={albumProfile?.notes?.text ?? ""}
        onClose={() => setAlbumNotesOpen(false)}
        />
      ) : null}

      {!showGate ? (
        <TrackAnalysisModal
        open={Boolean(analysisTarget)}
        trackId={analysisTarget?.trackId ?? null}
        title={analysisTarget?.title ?? ""}
        artist={analysisTarget?.artist ?? null}
        onClose={() => setAnalysisTarget(null)}
        />
      ) : null}

      {!showGate ? (
        <CatalogMetadataDialog
        open={catalogOpen}
        albumId={albumViewId}
        albumTitle={selectedAlbum?.title ?? ""}
        artistName={selectedAlbum?.artist ?? ""}
        onClose={() => setCatalogOpen(false)}
        onUpdated={({ album }) => {
          if (album) {
            setAlbumProfile(album);
          } else {
            loadCatalogProfiles(albumViewId, selectedAlbum?.artist_id ?? null);
          }
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
        canPlay={Boolean(sessionId && (activeOutputId || isLocalSession))}
        isPaused={Boolean(status?.paused)}
        onPause={handlePause}
        onPlayFrom={handleQueuePlayFrom}
        onClear={handleQueueClear}
        />
      ) : null}

      {!showGate ? <audio ref={audioRef} preload="auto" style={{ display: "none" }} /> : null}

    </div>
  );
}
