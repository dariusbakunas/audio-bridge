import type { QueueItem } from "../types";
import QueueModal from "../components/QueueModal";
import { action } from "@storybook/addon-actions";
import "../styles.css";

const items: QueueItem[] = [
  {
    kind: "track",
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

export default {
  title: "Queue/QueueModal"
};

export function Default() {
  return (
    <QueueModal open={true} items={items} onClose={action("close")} formatMs={formatMs} />
  );
}

export function Empty() {
  return (
    <QueueModal open={true} items={[]} onClose={action("close")} formatMs={formatMs} />
  );
}
