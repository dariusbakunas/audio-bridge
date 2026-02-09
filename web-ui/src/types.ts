export interface OutputInfo {
  id: string;
  name: string;
  kind: string;
  state: string;
  provider_name?: string | null;
  supported_rates?: { min_hz: number; max_hz: number } | null;
}

export interface StatusResponse {
  now_playing?: string | null;
  paused?: boolean | null;
  elapsed_ms?: number | null;
  duration_ms?: number | null;
  source_codec?: string | null;
  source_bit_depth?: number | null;
  container?: string | null;
  output_sample_format?: string | null;
  resampling?: boolean | null;
  resample_from_hz?: number | null;
  resample_to_hz?: number | null;
  sample_rate?: number | null;
  output_sample_rate?: number | null;
  channels?: number | null;
  output_device?: string | null;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  format?: string | null;
  bitrate_kbps?: number | null;
  buffered_frames?: number | null;
  buffer_capacity_frames?: number | null;
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

export interface TrackResolveResponse {
  album_id?: number | null;
}

export interface TrackMetadataResponse {
  path: string;
  title?: string | null;
  artist?: string | null;
  album?: string | null;
  album_artist?: string | null;
  year?: number | null;
  track_number?: number | null;
  disc_number?: number | null;
}

export interface AlbumMetadataResponse {
  album_id: number;
  title?: string | null;
  album_artist?: string | null;
  year?: number | null;
}

export interface AlbumMetadataUpdateResponse {
  album_id: number;
}

export type MusicBrainzMatchKind = "track" | "album";

export interface MusicBrainzMatchCandidate {
  recording_mbid?: string | null;
  release_mbid?: string | null;
  artist_mbid?: string | null;
  title: string;
  artist: string;
  release_title?: string | null;
  score?: number | null;
  year?: number | null;
}

export interface MusicBrainzMatchSearchResponse {
  items: MusicBrainzMatchCandidate[];
}

export type MetadataEvent =
  | {
      kind: "library_scan_album_start";
      path: string;
    }
  | {
      kind: "library_scan_album_finish";
      path: string;
      tracks: number;
    }
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

export interface LogEvent {
  level: string;
  target: string;
  message: string;
  timestamp_ms: number;
}
