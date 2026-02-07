import type { OutputInfo, StatusResponse } from "../types";
import PlayerBar from "../components/PlayerBar";
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

export function Default() {
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
        onCoverError={() => undefined}
        onAlbumNavigate={() => undefined}
        onPrimaryAction={() => undefined}
        onNext={() => undefined}
        onSignalOpen={() => undefined}
        onQueueOpen={() => undefined}
        onSelectOutput={() => undefined}
      />
    </div>
  );
}
