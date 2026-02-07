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
  metadataEvents,
  logEvents,
  logsError,
  rescanBusy,
  onClearMetadata,
  onRescanLibrary,
  onClearLogs,
  describeMetadataEvent,
  metadataDetailLines
}: SettingsViewProps) {
  return (
    <section className={`settings-screen ${active ? "active" : ""}`}>
      <div className="settings-stack">
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

        <div className="card">
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
                return (
                  <div key={entry.id} className="log-row">
                    <span className="log-time">{timestamp.toLocaleTimeString()}</span>
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
      </div>
    </section>
  );
}
