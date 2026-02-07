import type { AlbumSummary, TrackSummary } from "../types";
import AlbumDetailView from "../components/AlbumDetailView";
import { action } from "@storybook/addon-actions";
import cover1 from "./covers/cover-1.png";
import "../styles.css";

const placeholder = (title?: string | null, artist?: string | null) => {
  const text = `${title ?? ""}${artist ? ` ${artist}` : ""}`.trim() || "Album";
  const label = text
    .split(/\s+/)
    .map((part) => part[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="320"><rect width="100%" height="100%" fill="#50555b"/><text x="22" y="40" font-family="Space Grotesk, sans-serif" font-size="32" fill="#ffffff" text-anchor="start">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
};

const album: AlbumSummary = {
  id: 1,
  title: "In Rainbows",
  artist: "Radiohead",
  year: 2007,
  mbid: "9c48f1d6-4e4b-4d5c-9d6b-2e4c9845a13a",
  track_count: 10,
  cover_art_url: cover1
};

const tracks: TrackSummary[] = [
  {
    id: 1,
    path: "/music/Radiohead/In Rainbows/01 - 15 Step.flac",
    file_name: "01 - 15 Step.flac",
    title: "15 Step",
    artist: "Radiohead",
    track_number: 1,
    duration_ms: 224000
  },
  {
    id: 2,
    path: "/music/Radiohead/In Rainbows/02 - Bodysnatchers.flac",
    file_name: "02 - Bodysnatchers.flac",
    title: "Bodysnatchers",
    artist: "Radiohead",
    track_number: 2,
    duration_ms: 241000
  },
  {
    id: 3,
    path: "/music/Radiohead/In Rainbows/03 - Nude.flac",
    file_name: "03 - Nude.flac",
    title: "Nude",
    artist: "Radiohead",
    track_number: 3,
    duration_ms: 255000
  }
];

const formatMs = (ms?: number | null) => {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

export default {
  title: "Albums/AlbumDetailView"
};

export function Default() {
  return (
    <div style={{ padding: 24 }}>
      <AlbumDetailView
        album={album}
        tracks={tracks}
        loading={false}
        error={null}
        placeholder={placeholder}
        canPlay={true}
        formatMs={formatMs}
        activeAlbumId={1}
        isPlaying={true}
        isPaused={false}
        onPause={action("pause")}
        onPlayAlbum={action("play-album")}
        onPlayTrack={action("play-track")}
        onQueueTrack={action("queue-track")}
      />
    </div>
  );
}

export function EmptyTracks() {
  return (
    <div style={{ padding: 24 }}>
      <AlbumDetailView
        album={{ ...album, cover_art_url: null }}
        tracks={[]}
        loading={false}
        error={null}
        placeholder={placeholder}
        canPlay={false}
        formatMs={formatMs}
        activeAlbumId={null}
        isPlaying={false}
        isPaused={false}
        onPause={action("pause")}
        onPlayAlbum={action("play-album")}
        onPlayTrack={action("play-track")}
        onQueueTrack={action("queue-track")}
      />
    </div>
  );
}
