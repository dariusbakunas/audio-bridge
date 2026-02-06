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
