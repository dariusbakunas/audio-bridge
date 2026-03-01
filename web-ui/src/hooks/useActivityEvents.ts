import { useCallback, useRef, useState } from "react";

import { postJson } from "../api";
import { LogEvent, MetadataEvent } from "../types";
import { useLogsStream, useMetadataStream } from "./streams";

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

interface LogEventEntry {
  id: number;
  event: LogEvent;
}

const MAX_METADATA_EVENTS = 200;
const MAX_LOG_EVENTS = 300;

type UseActivityEventsArgs = {
  settingsOpen: boolean;
  serverConnected: boolean;
  settingsSection: string;
  connectionError: (summary: string, endpoint: string) => string;
  reportError: (message: string, severity?: "info" | "warn" | "error") => void;
};

export function useActivityEvents({
  settingsOpen,
  serverConnected,
  settingsSection,
  connectionError,
  reportError
}: UseActivityEventsArgs) {
  const [metadataEvents, setMetadataEvents] = useState<MetadataEventEntry[]>([]);
  const [logEvents, setLogEvents] = useState<LogEventEntry[]>([]);
  const [logsError, setLogsError] = useState<string | null>(null);
  const metadataIdRef = useRef(0);
  const logIdRef = useRef(0);

  const handleClearLogs = useCallback(async () => {
    setLogEvents([]);
    try {
      await postJson<{ cleared_at_ms: number }>("/logs/clear");
      setLogsError(null);
    } catch (err) {
      setLogsError((err as Error).message);
    }
  }, []);

  useMetadataStream({
    enabled: settingsOpen && serverConnected && settingsSection === "metadata",
    onEvent: (event) => {
      const entry: MetadataEventEntry = {
        id: (metadataIdRef.current += 1),
        time: new Date(),
        event
      };
      setMetadataEvents((prev) => [entry, ...prev].slice(0, MAX_METADATA_EVENTS));
    },
    onError: () =>
      reportError(connectionError("Live metadata updates disconnected", "/metadata/stream"), "warn")
  });

  useLogsStream({
    enabled: settingsOpen && serverConnected && settingsSection === "logs",
    onSnapshot: (items) => {
      const entries = items
        .map((entry) => ({
          id: (logIdRef.current += 1),
          event: entry
        }))
        .reverse()
        .slice(0, MAX_LOG_EVENTS);
      setLogEvents(entries);
      setLogsError(null);
    },
    onEvent: (entry) => {
      const row: LogEventEntry = {
        id: (logIdRef.current += 1),
        event: entry
      };
      setLogEvents((prev) => [row, ...prev].slice(0, MAX_LOG_EVENTS));
    },
    onError: () => {
      const message = connectionError("Live logs disconnected", "/logs/stream");
      setLogsError(message);
      reportError(message, "warn");
    }
  });

  return {
    metadataEvents,
    setMetadataEvents,
    logEvents,
    logsError,
    handleClearLogs
  };
}
