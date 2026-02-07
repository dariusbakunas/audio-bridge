import type { LogEvent, MetadataEvent } from "../types";
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
  title: "Settings/SettingsView"
};

export function Metadata() {
  return (
    <div style={{ padding: 24 }}>
      <SettingsView
        active={true}
        section="metadata"
        onSectionChange={action("section-change")}
        metadataEvents={metadataEvents}
        logEvents={logEvents}
        logsError={null}
        rescanBusy={false}
        onClearMetadata={action("clear-metadata")}
        onRescanLibrary={action("rescan-library")}
        onClearLogs={action("clear-logs")}
        describeMetadataEvent={describeMetadataEvent}
        metadataDetailLines={metadataDetailLines}
      />
    </div>
  );
}

export function Logs() {
  return (
    <div style={{ padding: 24 }}>
      <SettingsView
        active={true}
        section="logs"
        onSectionChange={action("section-change")}
        metadataEvents={metadataEvents}
        logEvents={logEvents}
        logsError={null}
        rescanBusy={false}
        onClearMetadata={action("clear-metadata")}
        onRescanLibrary={action("rescan-library")}
        onClearLogs={action("clear-logs")}
        describeMetadataEvent={describeMetadataEvent}
        metadataDetailLines={metadataDetailLines}
      />
    </div>
  );
}

export function LogsEmpty() {
  return (
    <div style={{ padding: 24 }}>
      <SettingsView
        active={true}
        section="logs"
        onSectionChange={action("section-change")}
        metadataEvents={[]}
        logEvents={[]}
        logsError={"Stream disconnected."}
        rescanBusy={false}
        onClearMetadata={action("clear-metadata")}
        onRescanLibrary={action("rescan-library")}
        onClearLogs={action("clear-logs")}
        describeMetadataEvent={describeMetadataEvent}
        metadataDetailLines={metadataDetailLines}
      />
    </div>
  );
}
