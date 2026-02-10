import type { AlbumSummary } from "../types";
import AlbumsView from "../components/AlbumsView";
import { action } from "@storybook/addon-actions";
// @ts-ignore
import cover1 from "./covers/cover-1.png";
// @ts-ignore
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
    cover_art_url: cover1,
    hi_res: true
  },
  {
    id: 2,
    title: "Promises",
    artist: "Floating Points",
    year: 2021,
    mbid: null,
    track_count: 9,
    cover_art_url: null,
    hi_res: false
  },
  {
    id: 3,
    title: "Blue Lines",
    artist: "Massive Attack",
    year: 1991,
    mbid: null,
    track_count: 9,
    cover_art_url: cover2,
    hi_res: false
  }
];

export default {
  title: "Albums/AlbumsView",
  component: AlbumsView,
  argTypes: {
    loading: { control: "boolean" },
    error: { control: "text" },
    canPlay: { control: "boolean" },
    activeAlbumId: { control: { type: "number", min: 0 } },
    isPlaying: { control: "boolean" },
    isPaused: { control: "boolean" }
  }
};

type AlbumsViewArgs = {
  loading: boolean;
  error: string;
  canPlay: boolean;
  activeAlbumId: number;
  isPlaying: boolean;
  isPaused: boolean;
};

const Template = (args: AlbumsViewArgs) => (
  <div style={{ padding: 24 }}>
    <AlbumsView
      albums={albums}
      loading={args.loading}
      error={args.error ? args.error : null}
      placeholder={placeholder}
      canPlay={args.canPlay}
      activeAlbumId={args.activeAlbumId > 0 ? args.activeAlbumId : null}
      isPlaying={args.isPlaying}
      isPaused={args.isPaused}
      onSelectAlbum={action("select-album")}
      onPlayAlbum={action("play-album")}
      onPause={action("pause")}
    />
  </div>
);

export const Default = Template.bind({});
// @ts-ignore
Default.args = {
  loading: false,
  error: "",
  canPlay: true,
  activeAlbumId: 1,
  isPlaying: true,
  isPaused: false
};
