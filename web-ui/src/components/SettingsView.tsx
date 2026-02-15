import { useEffect, useState } from "react";
import { LogEvent, MetadataEvent } from "../types";

interface MetadataEventEntry {
  id: number;
  time: Date;
  event: MetadataEvent;
}

interface LogEventEntry {
  id: number;
  event: LogEvent;
}

interface SettingsViewProps {
  active: boolean;
  section: "metadata" | "logs" | "connection";
  onSectionChange: (section: "metadata" | "logs" | "connection") => void;
  apiBase: string;
  apiBaseDefault: string;
  onApiBaseChange: (value: string) => void;
  onApiBaseReset: () => void;
  onReconnect: () => void;
  metadataEvents: MetadataEventEntry[];
  logEvents: LogEventEntry[];
  logsError: string | null;
  rescanBusy: boolean;
  onClearMetadata: () => void;
  onRescanLibrary: () => void;
  onClearLogs: () => void;
  describeMetadataEvent: (event: MetadataEvent) => { title: string; detail?: string };
  metadataDetailLines: (event: MetadataEvent) => string[];
}

export default function SettingsView({
  active,
  section,
  onSectionChange,
  metadataEvents,
  logEvents,
  logsError,
  rescanBusy,
  onClearMetadata,
  onRescanLibrary,
  onClearLogs,
  describeMetadataEvent,
  metadataDetailLines,
  apiBase,
  apiBaseDefault,
  onApiBaseChange,
  onApiBaseReset,
  onReconnect
}: SettingsViewProps) {
  const isMetadata = section === "metadata";
  const isLogs = section === "logs";
  const isConnection = section === "connection";
  const [apiBaseDraft, setApiBaseDraft] = useState(apiBase);

  useEffect(() => {
    setApiBaseDraft(apiBase);
  }, [apiBase]);

  const trimmedDraft = apiBaseDraft.trim();
  const isDirty = trimmedDraft !== apiBase.trim();
  const effectiveBase = apiBase.trim() || apiBaseDefault.trim();
  return (
    <section className={`settings-screen ${active ? "active" : ""}`}>
      <div className="settings-stack">
        <div className="settings-tabs">
          <button
            className={`settings-tab ${isMetadata ? "active" : ""}`}
            onClick={() => onSectionChange("metadata")}
          >
            Metadata
          </button>
          <button
            className={`settings-tab ${isConnection ? "active" : ""}`}
            onClick={() => onSectionChange("connection")}
          >
            Connection
          </button>
          <button
            className={`settings-tab ${isLogs ? "active" : ""}`}
            onClick={() => onSectionChange("logs")}
          >
            Logs
          </button>
        </div>

        {isMetadata ? (
          <div className="card">
            <div className="card-header">
              <span>Metadata jobs</span>
              <div className="card-actions">
                <button className="btn ghost small" onClick={onClearMetadata}>
                  Clear
                </button>
                <span className="pill">{metadataEvents.length} events</span>
              </div>
            </div>
            <div className="settings-panel">
              <div className="muted small">Live MusicBrainz and cover art updates.</div>
              <div className="settings-actions">
                <button className="btn ghost small" onClick={onRescanLibrary} disabled={rescanBusy}>
                  {rescanBusy ? "Rescanning..." : "Rescan library"}
                </button>
              </div>
              <div className="settings-list">
                {metadataEvents.map((entry) => {
                  const info = describeMetadataEvent(entry.event);
                  const extraLines = metadataDetailLines(entry.event);
                  return (
                    <div key={entry.id} className="settings-row">
                      <div>
                        <div className="settings-title">{info.title}</div>
                        <div className="muted small">{info.detail ?? "—"}</div>
                        {extraLines.map((line) => (
                          <div key={line} className="muted small">
                            {line}
                          </div>
                        ))}
                      </div>
                      <div className="muted small">{entry.time.toLocaleTimeString()}</div>
                    </div>
                  );
                })}
                {metadataEvents.length === 0 ? (
                  <div className="muted small">No metadata events yet.</div>
                ) : null}
              </div>
            </div>
          </div>
        ) : null}

        {isLogs ? (
          <div className="logs-panel">
            <div className="card-header">
              <span>Server logs</span>
              <div className="card-actions">
                <button className="btn ghost small" onClick={onClearLogs}>
                  Clear
                </button>
                <span className="pill">{logEvents.length} lines</span>
              </div>
            </div>
            <div className="settings-panel">
              <div className="muted small">Live log stream from the hub server.</div>
              {logsError ? <div className="muted small">{logsError}</div> : null}
              <div className="log-list">
                {logEvents.map((entry) => {
                  const timestamp = new Date(entry.event.timestamp_ms);
                  const level = entry.event.level.toLowerCase();
                  const date = new Date(entry.event.timestamp_ms);
                  const dateLabel = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
                  const timeLabel = date.toLocaleTimeString([], { hour12: false });
                  return (
                    <div key={entry.id} className="log-row">
                      <span className="log-time">{dateLabel} {timeLabel}</span>
                      <span className={`log-level log-${level}`}>{entry.event.level}</span>
                      <span className="log-message">
                        {entry.event.message}
                        <span className="log-target">{entry.event.target}</span>
                      </span>
                    </div>
                  );
                })}
                {logEvents.length === 0 ? <div className="muted small">No logs yet.</div> : null}
              </div>
            </div>
          </div>
        ) : null}

        {isConnection ? (
          <div className="card">
            <div className="card-header">
              <span>Hub server</span>
              <div className="card-actions">
                <button
                  className="btn ghost small"
                  onClick={() => {
                    setApiBaseDraft("");
                    onApiBaseReset();
                  }}
                  disabled={!apiBase.trim() && !apiBaseDraft.trim()}
                >
                  Reset
                </button>
              </div>
            </div>
            <div className="settings-panel">
              <div className="muted small">
                Set the base URL for the hub server (example: http://192.168.1.10:8080).
                Leave blank to use the default.
              </div>
              <div className="muted small">
                After saving, refresh the app to reconnect live streams.
              </div>
              <div className="settings-field">
                <label className="settings-label" htmlFor="api-base-input">
                  API base URL
                </label>
                <input
                  id="api-base-input"
                  className="settings-input"
                  type="url"
                  placeholder={apiBaseDefault || "http://<SERVER_IP>:8080"}
                  value={apiBaseDraft}
                  onChange={(event) => setApiBaseDraft(event.target.value)}
                />
              </div>
              <div className="settings-actions">
                <button className="btn ghost small" onClick={onReconnect}>
                  Reconnect now
                </button>
                <button
                  className="btn ghost small"
                  onClick={() => onApiBaseChange(apiBaseDraft)}
                  disabled={!isDirty}
                >
                  Save
                </button>
              </div>
              <div className="muted small">
                Effective base: {effectiveBase ? effectiveBase : "current origin"}
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </section>
  );
}
