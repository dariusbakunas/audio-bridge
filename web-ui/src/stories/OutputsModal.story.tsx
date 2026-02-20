import type { OutputInfo } from "../types";
import OutputsModal from "../components/OutputsModal";
import { action } from "storybook/actions";
import "../styles.css";

const outputs: OutputInfo[] = [
  {
    id: "bridge:living-room:default",
    name: "Living Room",
    kind: "bridge",
    state: "online",
    provider_name: "roon-bridge",
    supported_rates: { min_hz: 44100, max_hz: 192000 }
  },
  {
    id: "bridge:studio:usb",
    name: "Studio DAC",
    kind: "bridge",
    state: "online",
    provider_name: "alsa",
    supported_rates: { min_hz: 48000, max_hz: 384000 }
  },
  {
    id: "bridge:kitchen:airplay",
    name: "Kitchen",
    kind: "bridge",
    state: "offline",
    provider_name: "airplay",
    supported_rates: null
  }
];

const formatRateRange = (output: OutputInfo) => {
  if (!output.supported_rates) return "rate: unknown";
  const min = Math.round(output.supported_rates.min_hz / 1000);
  const max = Math.round(output.supported_rates.max_hz / 1000);
  return `${min}-${max} kHz`;
};

export default {
  title: "Outputs/OutputsModal",
  component: OutputsModal,
  argTypes: {
    open: { control: "boolean" },
    activeOutputId: { control: "text" },
    showEmpty: { control: "boolean" }
  }
};

type OutputsModalArgs = {
  open: boolean;
  activeOutputId: string;
  showEmpty: boolean;
};

const Template = (args: OutputsModalArgs) => (
  <OutputsModal
    open={args.open}
    outputs={args.showEmpty ? [] : outputs}
    activeOutputId={args.activeOutputId || null}
    onClose={action("close")}
    onSelectOutput={action("select-output")}
    formatRateRange={formatRateRange}
  />
);

export const Default = Template.bind({});
Default.args = {
  open: true,
  activeOutputId: "bridge:living-room:default",
  showEmpty: false
};

export const Empty = Template.bind({});
Empty.args = {
  open: true,
  activeOutputId: "",
  showEmpty: true
};
