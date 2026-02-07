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
  title: "Player/PlayerBar"
};

export function Playing() {
  return (
    <div style={{ paddingBottom: 120 }}>
      <PlayerBar
        status={status}
        nowPlayingCover={null}
        nowPlayingCoverFailed={true}
        placeholderCover={placeholderCover}
        isPlaying={true}
        canTogglePlayback={true}
        showPlayIcon={false}
        playButtonTitle={undefined}
        queueHasItems={true}
        activeOutput={activeOutput}
        activeAlbumId={1}
        uiBuildId="dev"
        formatMs={(ms) => {
          if (!ms && ms !== 0) return "--:--";
          const totalSeconds = Math.floor(ms / 1000);
          const minutes = Math.floor(totalSeconds / 60);
          const seconds = totalSeconds % 60;
          return `${minutes}:${seconds.toString().padStart(2, "0")}`;
        }}
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
}

export function Paused() {
  return (
    <div style={{ paddingBottom: 120 }}>
      <PlayerBar
        status={{ ...status, paused: true }}
        nowPlayingCover={null}
        nowPlayingCoverFailed={true}
        placeholderCover={placeholderCover}
        isPlaying={false}
        canTogglePlayback={true}
        showPlayIcon={true}
        playButtonTitle={undefined}
        queueHasItems={true}
        activeOutput={activeOutput}
        activeAlbumId={1}
        uiBuildId="dev"
        formatMs={(ms) => {
          if (!ms && ms !== 0) return "--:--";
          const totalSeconds = Math.floor(ms / 1000);
          const minutes = Math.floor(totalSeconds / 60);
          const seconds = totalSeconds % 60;
          return `${minutes}:${seconds.toString().padStart(2, "0")}`;
        }}
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
}

export function NothingPlaying() {
  return (
    <div style={{ paddingBottom: 120 }}>
      <PlayerBar
        status={null}
        nowPlayingCover={null}
        nowPlayingCoverFailed={true}
        placeholderCover={placeholderCover}
        isPlaying={false}
        canTogglePlayback={false}
        showPlayIcon={true}
        playButtonTitle="Select a track to play."
        queueHasItems={false}
        activeOutput={activeOutput}
        activeAlbumId={null}
        uiBuildId="dev"
        formatMs={(ms) => {
          if (!ms && ms !== 0) return "--:--";
          const totalSeconds = Math.floor(ms / 1000);
          const minutes = Math.floor(totalSeconds / 60);
          const seconds = totalSeconds % 60;
          return `${minutes}:${seconds.toString().padStart(2, "0")}`;
        }}
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
}

export function NoNextTrack() {
  return (
    <div style={{ paddingBottom: 120 }}>
      <PlayerBar
        status={status}
        nowPlayingCover={null}
        nowPlayingCoverFailed={true}
        placeholderCover={placeholderCover}
        isPlaying={true}
        canTogglePlayback={true}
        showPlayIcon={false}
        playButtonTitle={undefined}
        queueHasItems={false}
        activeOutput={activeOutput}
        activeAlbumId={1}
        uiBuildId="dev"
        formatMs={(ms) => {
          if (!ms && ms !== 0) return "--:--";
          const totalSeconds = Math.floor(ms / 1000);
          const minutes = Math.floor(totalSeconds / 60);
          const seconds = totalSeconds % 60;
          return `${minutes}:${seconds.toString().padStart(2, "0")}`;
        }}
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
}

export function NoOutputSelected() {
  return (
    <div style={{ paddingBottom: 120 }}>
      <PlayerBar
        status={status}
        nowPlayingCover={null}
        nowPlayingCoverFailed={true}
        placeholderCover={placeholderCover}
        isPlaying={true}
        canTogglePlayback={false}
        showPlayIcon={false}
        playButtonTitle="Select an output to control playback."
        queueHasItems={false}
        activeOutput={null}
        activeAlbumId={1}
        uiBuildId="dev"
        formatMs={(ms) => {
          if (!ms && ms !== 0) return "--:--";
          const totalSeconds = Math.floor(ms / 1000);
          const minutes = Math.floor(totalSeconds / 60);
          const seconds = totalSeconds % 60;
          return `${minutes}:${seconds.toString().padStart(2, "0")}`;
        }}
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
}
