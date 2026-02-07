import type { OutputInfo, StatusResponse } from "../types";
import PlayerBar from "../components/PlayerBar";
import { action } from "@storybook/addon-actions";
import "../styles.css";

const status: StatusResponse = {
  now_playing: "/music/Radiohead/In Rainbows/01 - 15 Step.flac",
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
    isPlaying: { control: "boolean" },
    canTogglePlayback: { control: "boolean" },
    showPlayIcon: { control: "boolean" },
    queueHasItems: { control: "boolean" },
    activeAlbumId: { control: { type: "number", min: 0 } },
    hasOutput: { control: "boolean" },
    hasStatus: { control: "boolean" },
    nowPlayingCoverFailed: { control: "boolean" },
    playButtonTitle: { control: "text" }
  }
};

type PlayerBarArgs = {
  isPlaying: boolean;
  canTogglePlayback: boolean;
  showPlayIcon: boolean;
  queueHasItems: boolean;
  activeAlbumId: number;
  hasOutput: boolean;
  hasStatus: boolean;
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
      status={args.hasStatus ? status : null}
      nowPlayingCover={null}
      nowPlayingCoverFailed={args.nowPlayingCoverFailed}
      placeholderCover={placeholderCover}
      isPlaying={args.isPlaying}
      canTogglePlayback={args.canTogglePlayback}
      showPlayIcon={args.showPlayIcon}
      playButtonTitle={args.playButtonTitle || undefined}
      queueHasItems={args.queueHasItems}
      activeOutput={args.hasOutput ? activeOutput : null}
      activeAlbumId={args.activeAlbumId > 0 ? args.activeAlbumId : null}
      uiBuildId="dev"
      formatMs={formatMs}
      onCoverError={action("cover-error")}
      onAlbumNavigate={action("navigate-album")}
      onPrimaryAction={action("primary-action")}
      onNext={action("next")}
      onSignalOpen={action("signal-open")}
      onQueueOpen={action("queue-open")}
      onSelectOutput={action("select-output")}
    />
  </div>
);

export const Playing = Template.bind({});
Playing.args = {
  isPlaying: true,
  canTogglePlayback: true,
  showPlayIcon: false,
  queueHasItems: true,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const Paused = Template.bind({});
Paused.args = {
  isPlaying: false,
  canTogglePlayback: true,
  showPlayIcon: true,
  queueHasItems: true,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const NothingPlaying = Template.bind({});
NothingPlaying.args = {
  isPlaying: false,
  canTogglePlayback: false,
  showPlayIcon: true,
  queueHasItems: false,
  activeAlbumId: 0,
  hasOutput: true,
  hasStatus: false,
  nowPlayingCoverFailed: true,
  playButtonTitle: "Select a track to play."
};

export const NoNextTrack = Template.bind({});
NoNextTrack.args = {
  isPlaying: true,
  canTogglePlayback: true,
  showPlayIcon: false,
  queueHasItems: false,
  activeAlbumId: 1,
  hasOutput: true,
  hasStatus: true,
  nowPlayingCoverFailed: true,
  playButtonTitle: ""
};

export const NoOutputSelected = Template.bind({});
NoOutputSelected.args = {
  isPlaying: true,
  canTogglePlayback: false,
  showPlayIcon: false,
  queueHasItems: false,
  activeAlbumId: 1,
  hasOutput: false,
  hasStatus: true,
  nowPlayingCoverFailed: true,
  playButtonTitle: "Select an output to control playback."
};
