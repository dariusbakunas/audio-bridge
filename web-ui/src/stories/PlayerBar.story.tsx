import type { OutputInfo, StatusResponse } from "../types";
import PlayerBar from "../components/PlayerBar";
import { action } from "storybook/actions";
import "../styles.css";

const status: StatusResponse = {
  now_playing_track_id: 101,
  paused: false,
  elapsed_ms: 63210,
  duration_ms: 224000,
  title: "15 Step",
  artist: "Radiohead",
  album: "In Rainbows",
  format: "FLAC",
  sample_rate: 44100,
  output_sample_rate: 44100,
  channels: 2
};

const activeOutput: OutputInfo = {
  id: "bridge:test:default",
  name: "Living Room",
  kind: "bridge",
  state: "online",
  provider_name: "roon-bridge",
  supported_rates: { min_hz: 44100, max_hz: 192000 }
};

const placeholderCover =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    "<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='100%' height='100%' fill='#50555b'/><text x='10' y='22' font-family='Space Grotesk, sans-serif' font-size='14' fill='#fff'>IR</text></svg>"
  );

export default {
  title: "Player/PlayerBar",
  component: PlayerBar,
  argTypes: {
    showSignalPath: { control: "boolean" },
    canTogglePlayback: { control: "boolean" },
    queueHasItems: { control: "boolean" },
    queueOpen: { control: "boolean" },
    activeAlbumId: { control: { type: "number", min: 0 } },
    hasOutput: { control: "boolean" },
    hasStatus: { control: "boolean" },
    paused: { control: "boolean" },
    nowPlayingCoverFailed: { control: "boolean" },
    playButtonTitle: { control: "text" }
  }
};

type PlayerBarArgs = {
  showSignalPath: boolean;
  canTogglePlayback: boolean;
  queueHasItems: boolean;
  queueOpen: boolean;
  activeAlbumId: number;
  hasOutput: boolean;
  hasStatus: boolean;
  paused: boolean;
  nowPlayingCoverFailed: boolean;
  playButtonTitle: string;
};

const formatMs = (ms?: number | null) => {
  if (!ms && ms !== 0) return "--:--";
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
};

const Template = (args: PlayerBarArgs) => (
  <div style={{ paddingBottom: 120 }}>
    <PlayerBar
      status={args.hasStatus ? { ...status, paused: args.paused } : null}
      nowPlayingCover={null}
      nowPlayingCoverFailed={args.nowPlayingCoverFailed}
      placeholderCover={placeholderCover}
      showSignalPath={args.showSignalPath}
      canTogglePlayback={args.canTogglePlayback}
      playButtonTitle={args.playButtonTitle || undefined}
      queueHasItems={args.queueHasItems}
      queueOpen={args.queueOpen}
      activeOutput={args.hasOutput ? activeOutput : null}
      activeAlbumId={args.activeAlbumId > 0 ? args.activeAlbumId : null}
      uiBuildId="dev"
      formatMs={formatMs}
      onCoverError={action("cover-error")}
      onAlbumNavigate={action("navigate-album")}
      onPrimaryAction={action("primary-action")}
      onPrevious={action("previous")}
      onNext={action("next")}
      onSignalOpen={action("signal-open")}
      onQueueOpen={action("queue-open")}
      onSelectOutput={action("select-output")}
    />
  </div>
);

export const Playing = Template.bind({});
Playing.args = {
  showSignalPath: true,
  canTogglePlayback: true,
  queueHasItems: true,
  queueOpen: false,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  paused: false,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const Paused = Template.bind({});
Paused.args = {
  showSignalPath: false,
  canTogglePlayback: true,
  queueHasItems: true,
  queueOpen: false,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  paused: true,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const NothingPlaying = Template.bind({});
NothingPlaying.args = {
  showSignalPath: false,
  canTogglePlayback: false,
  queueHasItems: false,
  queueOpen: false,
  activeAlbumId: 0,
  hasOutput: true,
  hasStatus: false,
  paused: false,
  nowPlayingCoverFailed: true,
  playButtonTitle: "Select a track to play."
};

export const NoNextTrack = Template.bind({});
NoNextTrack.args = {
  showSignalPath: true,
  canTogglePlayback: true,
  queueHasItems: false,
  queueOpen: false,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  paused: false,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const NoOutputSelected = Template.bind({});
NoOutputSelected.args = {
  showSignalPath: true,
  canTogglePlayback: false,
  queueHasItems: false,
  queueOpen: false,
  activeAlbumId: 1,
  hasOutput: false,
  hasStatus: true,
  paused: false,
  nowPlayingCoverFailed: true,
  playButtonTitle: "Select an output to control playback."
};
