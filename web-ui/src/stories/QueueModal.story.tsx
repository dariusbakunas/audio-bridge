import type { QueueItem } from "../types";
import QueueModal from "../components/QueueModal";
import { action } from "@storybook/addon-actions";
import "../styles.css";

const items: QueueItem[] = [
  {
    kind: "track",
    id: 101,
    path: "/music/Radiohead/In Rainbows/01 - 15 Step.flac",
    file_name: "01 - 15 Step.flac",
    duration_ms: 224000,
    sample_rate: 44100,
    album: "In Rainbows",
    artist: "Radiohead",
    format: "FLAC"
  },
  {
    kind: "track",
    id: 102,
    path: "/music/Radiohead/In Rainbows/02 - Bodysnatchers.flac",
    file_name: "02 - Bodysnatchers.flac",
    duration_ms: 241000,
    sample_rate: 44100,
    album: "In Rainbows",
    artist: "Radiohead",
    format: "FLAC"
  },
  {
    kind: "missing",
    path: "/music/Unknown/ghost.flac"
  }
];

const formatMs = (ms?: number | null) => {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

const placeholder = (title?: string | null, artist?: string | null) => {
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
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="#50555b"/><stop offset="100%" stop-color="#3f444a"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="18" y="32" font-family="Space Grotesk, sans-serif" font-size="28" fill="#ffffff" text-anchor="start">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
};

export default {
  title: "Queue/QueueModal"
};

export function Default() {
  return (
    <QueueModal
      open={true}
      items={items}
      onClose={action("close")}
      formatMs={formatMs}
      placeholder={placeholder}
      canPlay={true}
      onPlayFrom={action("play-from")}
    />
  );
}

export function Empty() {
  return (
    <QueueModal
      open={true}
      items={[]}
      onClose={action("close")}
      formatMs={formatMs}
      placeholder={placeholder}
      canPlay={false}
      onPlayFrom={action("play-from")}
    />
  );
}
