import type { OutputInfo, StatusResponse } from "../types";
import SignalModal from "../components/SignalModal";
import { action } from "@storybook/addon-actions";
import "../styles.css";

const activeOutput: OutputInfo = {
  id: "bridge:living-room:default",
  name: "Living Room",
  kind: "bridge",
  state: "online",
  provider_name: "roon-bridge",
  supported_rates: { min_hz: 44100, max_hz: 192000 }
};

const status: StatusResponse = {
  source_codec: "FLAC",
  source_bit_depth: 16,
  sample_rate: 44100,
  output_sample_rate: 96000,
  resampling: true,
  resample_to_hz: 96000,
  output_sample_format: "s32",
  channels: 2,
  bitrate_kbps: 905,
  buffered_frames: 2048,
  buffer_capacity_frames: 4096
};

const formatHz = (hz?: number | null) => {
  if (!hz) return "—";
  if (hz >= 1000) return `${(hz / 1000).toFixed(1)} kHz`;
  return `${hz} Hz`;
};

export default {
  title: "Player/SignalModal"
};

export function Playing() {
  return (
    <SignalModal
      open={true}
      status={status}
      activeOutput={activeOutput}
      updatedAt={new Date()}
      formatHz={formatHz}
      onClose={action("close")}
    />
  );
}

export function Idle() {
  return (
    <SignalModal
      open={true}
      status={null}
      activeOutput={null}
      updatedAt={null}
      formatHz={formatHz}
      onClose={action("close")}
    />
  );
}
