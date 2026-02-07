import type { AlbumSummary } from "../types";
import AlbumsView from "../components/AlbumsView";
import cover1 from "./covers/cover-1.png";
import cover2 from "./covers/cover-2.png";
import "../styles.css";

const placeholder = (title?: string | null, artist?: string | null) => {
  const text = `${title ?? ""}${artist ? ` ${artist}` : ""}`.trim() || "Album";
  const label = text
    .split(/\s+/)
    .map((part) => part[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><rect width="100%" height="100%" fill="#50555b"/><text x="18" y="32" font-family="Space Grotesk, sans-serif" font-size="28" fill="#ffffff" text-anchor="start">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
};

const albums: AlbumSummary[] = [
  {
    id: 1,
    title: "In Rainbows",
    artist: "Radiohead",
    year: 2007,
    mbid: null,
    track_count: 10,
    cover_art_url: cover1
  },
  {
    id: 2,
    title: "Promises",
    artist: "Floating Points",
    year: 2021,
    mbid: null,
    track_count: 9,
    cover_art_url: null
  },
  {
    id: 3,
    title: "Blue Lines",
    artist: "Massive Attack",
    year: 1991,
    mbid: null,
    track_count: 9,
    cover_art_url: cover2
  }
];

export default {
  title: "Albums/AlbumsView"
};

export function Default() {
  return (
    <div style={{ padding: 24 }}>
      <AlbumsView
        albums={albums}
        loading={false}
        error={null}
        placeholder={placeholder}
        canPlay={true}
        activeAlbumId={1}
        isPlaying={true}
        isPaused={false}
        onSelectAlbum={() => undefined}
        onPlayAlbum={() => undefined}
        onPause={() => undefined}
      />
    </div>
  );
}
