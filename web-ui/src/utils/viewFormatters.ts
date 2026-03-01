import { MetadataEvent, OutputInfo } from "../types";

export function albumPlaceholder(title?: string | null, artist?: string | null): string {
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

export function formatMs(ms?: number | null): string {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function formatHz(hz?: number | null): string {
  if (!hz) return "—";
  if (hz >= 1000) {
    return `${(hz / 1000).toFixed(1)} kHz`;
  }
  return `${hz} Hz`;
}

export function formatRateRange(output: OutputInfo): string {
  if (!output.supported_rates) return "rate range unknown";
  return `${formatHz(output.supported_rates.min_hz)} - ${formatHz(output.supported_rates.max_hz)}`;
}

export function normalizeMatch(value?: string | null): string {
  return value?.trim().toLowerCase() ?? "";
}

export function describeMetadataEvent(event: MetadataEvent): { title: string; detail?: string } {
  switch (event.kind) {
    case "library_scan_album_start":
      return { title: "Scanning album folder", detail: event.album };
    case "library_scan_album_finish":
      return { title: "Scanned album folder", detail: `${event.tracks} tracks` };
    case "music_brainz_batch":
      return { title: "MusicBrainz batch", detail: `${event.count} candidates` };
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
      return { title: "MusicBrainz lookup failed", detail: event.error };
    case "cover_art_batch":
      return { title: "Cover art batch", detail: `${event.count} albums` };
    case "cover_art_fetch_start":
      return { title: "Cover art fetch started", detail: `album ${event.album_id}` };
    case "cover_art_fetch_success":
      return { title: "Cover art fetched", detail: `album ${event.album_id}` };
    case "cover_art_fetch_failure":
      return {
        title: "Cover art fetch failed",
        detail: `${event.error} (attempt ${event.attempts})`
      };
    default:
      return { title: "Metadata event" };
  }
}

export function metadataDetailLines(event: MetadataEvent): string[] {
  if (event.kind !== "music_brainz_lookup_no_match") {
    if (event.kind === "library_scan_album_finish") {
      return [event.album];
    }
    if (event.kind === "library_scan_album_start") {
      return [event.album];
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
