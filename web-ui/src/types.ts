export interface OutputInfo {
  id: string;
  name: string;
  kind: string;
  state: string;
  provider_name?: string | null;
  supported_rates?: { min_hz: number; max_hz: number } | null;
}

export interface OutputsResponse {
  active_id: string | null;
  outputs: OutputInfo[];
}

export interface QueueItemTrack {
  kind: "track";
  path: string;
  file_name: string;
  duration_ms?: number | null;
  sample_rate?: number | null;
  album?: string | null;
  artist?: string | null;
  format: string;
}

export interface QueueItemMissing {
  kind: "missing";
  path: string;
}

export type QueueItem = QueueItemTrack | QueueItemMissing;

export interface QueueResponse {
  items: QueueItem[];
}

export interface LibraryEntryDir {
  kind: "dir";
  path: string;
  name: string;
}

export interface LibraryEntryTrack {
  kind: "track";
  path: string;
  file_name: string;
  duration_ms?: number | null;
  sample_rate?: number | null;
  album?: string | null;
  artist?: string | null;
  format: string;
}

export type LibraryEntry = LibraryEntryDir | LibraryEntryTrack;

export interface LibraryResponse {
  dir: string;
  entries: LibraryEntry[];
}

export interface AlbumSummary {
  id: number;
  title: string;
  artist?: string | null;
  year?: number | null;
  mbid?: string | null;
  track_count: number;
  cover_art_path?: string | null;
  cover_art_url?: string | null;
}

export interface TrackSummary {
  id: number;
  path: string;
  file_name: string;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  track_number?: number | null;
  disc_number?: number | null;
  duration_ms?: number | null;
  format?: string | null;
  mbid?: string | null;
  cover_art_url?: string | null;
}

export interface AlbumListResponse {
  items: AlbumSummary[];
}

export interface TrackListResponse {
  items: TrackSummary[];
}

export type MetadataEvent =
  | {
      kind: "music_brainz_batch";
      count: number;
    }
  | {
      kind: "music_brainz_lookup_start";
      path: string;
      title: string;
      artist: string;
      album?: string | null;
    }
  | {
      kind: "music_brainz_lookup_success";
      path: string;
      recording_mbid?: string | null;
      artist_mbid?: string | null;
      album_mbid?: string | null;
    }
  | {
      kind: "music_brainz_lookup_no_match";
      path: string;
      title: string;
      artist: string;
      album?: string | null;
      query: string;
      top_score?: number | null;
      best_recording_id?: string | null;
      best_recording_title?: string | null;
    }
  | {
      kind: "music_brainz_lookup_failure";
      path: string;
      error: string;
    }
  | {
      kind: "cover_art_batch";
      count: number;
    }
  | {
      kind: "cover_art_fetch_start";
      album_id: number;
      mbid: string;
    }
  | {
      kind: "cover_art_fetch_success";
      album_id: number;
      cover_path: string;
    }
  | {
      kind: "cover_art_fetch_failure";
      album_id: number;
      mbid: string;
      error: string;
      attempts: number;
    };
