import type { LogEvent, MetadataEvent, OutputSettings, ProviderOutputs } from "../types";
import SettingsView from "../components/SettingsView";
import { action } from "@storybook/addon-actions";
import "../styles.css";

const metadataEvents = [
  {
    id: 1,
    time: new Date(Date.now() - 1000 * 60 * 2),
    event: {
      kind: "music_brainz_lookup_start",
      path: "/music/Radiohead/In Rainbows/01 - 15 Step.flac",
      title: "15 Step",
      artist: "Radiohead",
      album: "In Rainbows"
    } as MetadataEvent
  },
  {
    id: 2,
    time: new Date(Date.now() - 1000 * 60),
    event: {
      kind: "cover_art_fetch_success",
      album_id: 42,
      cover_path: "/covers/42.jpg"
    } as MetadataEvent
  }
];

const logEvents = [
  {
    id: 1,
    event: {
      level: "INFO",
      target: "audio_hub_server::startup",
      message: "Server started on 0.0.0.0:8080",
      timestamp_ms: Date.now() - 1000 * 20
    } as LogEvent
  },
  {
    id: 2,
    event: {
      level: "WARN",
      target: "audio_hub_server::output_providers::bridge_provider",
      message: "Output device went offline",
      timestamp_ms: Date.now() - 1000 * 5
    } as LogEvent
  }
];

const outputSettings: OutputSettings = {
  disabled: ["bridge:living-room:device-2"],
  renames: {
    "bridge:living-room:device-1": "Living Room DAC"
  },
  exclusive: ["bridge:living-room:device-1"]
};

const outputProviders: ProviderOutputs[] = [
  {
    provider: {
      id: "bridge:living-room",
      kind: "bridge",
      name: "Living Room Bridge",
      state: "connected",
      capabilities: { device_select: true, volume: false }
    },
    address: "192.168.1.50:5556",
    outputs: [
      {
        id: "bridge:living-room:device-1",
        kind: "bridge",
        name: "Built-in Output",
        state: "online",
        provider_name: "Living Room Bridge"
      },
      {
        id: "bridge:living-room:device-2",
        kind: "bridge",
        name: "USB DAC",
        state: "online",
        provider_name: "Living Room Bridge"
      }
    ]
  },
  {
    provider: {
      id: "local:host",
      kind: "local",
      name: "Local Host",
      state: "available",
      capabilities: { device_select: true, volume: false }
    },
    outputs: [
      {
        id: "local:host:default",
        kind: "local",
        name: "Built-in Audio",
        state: "online",
        provider_name: "Local Host"
      }
    ]
  }
];

const describeMetadataEvent = (event: MetadataEvent) => {
  switch (event.kind) {
    case "music_brainz_lookup_start":
      return { title: "Lookup started", detail: event.title };
    case "cover_art_fetch_success":
      return { title: "Cover art cached", detail: event.cover_path };
    default:
      return { title: "Metadata event", detail: event.kind };
  }
};

const metadataDetailLines = (event: MetadataEvent) => {
  if (event.kind === "music_brainz_lookup_start") {
    return [event.path, event.artist];
  }
  return [];
};

export default {
  title: "Settings/SettingsView",
  component: SettingsView,
  argTypes: {
    section: { control: { type: "radio" }, options: ["metadata", "connection", "outputs", "logs"] },
    logsError: { control: "text" },
    rescanBusy: { control: "boolean" },
    empty: { control: "boolean" }
  }
};

type SettingsViewArgs = {
  section: "metadata" | "connection" | "outputs" | "logs";
  logsError: string;
  rescanBusy: boolean;
  empty: boolean;
};

const Template = (args: SettingsViewArgs) => (
  <div style={{ padding: 24 }}>
    <SettingsView
      active={true}
      section={args.section}
      onSectionChange={action("section-change")}
      apiBase="http://192.168.1.10:8080"
      apiBaseDefault=""
      onApiBaseChange={action("api-base-change")}
      onApiBaseReset={action("api-base-reset")}
      onReconnect={action("reconnect")}
      outputsSettings={outputSettings}
      outputsProviders={args.empty ? [] : outputProviders}
      outputsLoading={false}
      outputsError={null}
      outputsLastRefresh={{}}
      onRefreshProvider={action("refresh-provider")}
      onToggleOutput={action("toggle-output")}
      onRenameOutput={action("rename-output")}
      onToggleExclusive={action("toggle-exclusive")}
      metadataEvents={args.empty ? [] : metadataEvents}
      logEvents={args.empty ? [] : logEvents}
      logsError={args.logsError || null}
      rescanBusy={args.rescanBusy}
      onClearMetadata={action("clear-metadata")}
      onRescanLibrary={action("rescan-library")}
      onClearLogs={action("clear-logs")}
      describeMetadataEvent={describeMetadataEvent}
      metadataDetailLines={metadataDetailLines}
    />
  </div>
);

export const Metadata = Template.bind({});
Metadata.args = {
  section: "metadata",
  logsError: "",
  rescanBusy: false,
  empty: false
};

export const Logs = Template.bind({});
Logs.args = {
  section: "logs",
  logsError: "",
  rescanBusy: false,
  empty: false
};

export const LogsEmpty = Template.bind({});
LogsEmpty.args = {
  section: "logs",
  logsError: "Stream disconnected.",
  rescanBusy: false,
  empty: true
};

export const Connection = Template.bind({});
Connection.args = {
  section: "connection",
  logsError: "",
  rescanBusy: false,
  empty: true
};

export const Outputs = Template.bind({});
Outputs.args = {
  section: "outputs",
  logsError: "",
  rescanBusy: false,
  empty: false
};
