import { useEffect, useState } from "react";
import { RefreshCw, Edit2, Circle } from "lucide-react";
import { LogEvent, MetadataEvent, OutputSettings, ProviderOutputs } from "../types";

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
  section: "metadata" | "logs" | "connection" | "outputs";
  onSectionChange: (section: "metadata" | "logs" | "connection" | "outputs") => void;
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
  outputsSettings: OutputSettings | null;
  outputsProviders: ProviderOutputs[];
  outputsLoading: boolean;
  outputsError: string | null;
  outputsLastRefresh: Record<string, string>;
  onRefreshProvider: (providerId: string) => void;
  onToggleOutput: (outputId: string, enabled: boolean) => void;
  onRenameOutput: (outputId: string, name: string) => void;
  onToggleExclusive: (outputId: string, enabled: boolean) => void;
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
  onReconnect,
  outputsSettings,
  outputsProviders,
  outputsLoading,
  outputsError,
  outputsLastRefresh,
  onRefreshProvider,
  onToggleOutput,
  onRenameOutput,
  onToggleExclusive
}: SettingsViewProps) {
  const isMetadata = section === "metadata";
  const isLogs = section === "logs";
  const isConnection = section === "connection";
  const isOutputs = section === "outputs";
  const [apiBaseDraft, setApiBaseDraft] = useState(apiBase);
  const [renamingOutput, setRenamingOutput] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  useEffect(() => {
    setApiBaseDraft(apiBase);
  }, [apiBase]);

  const trimmedDraft = apiBaseDraft.trim();
  const isDirty = trimmedDraft !== apiBase.trim();
  const effectiveBase = apiBase.trim() || apiBaseDefault.trim();

  const resolvedName = (outputId: string, fallback: string) => {
    if (!outputsSettings) return fallback;
    return outputsSettings.renames[outputId] ?? fallback;
  };
  const isEnabled = (outputId: string) => {
    if (!outputsSettings) return true;
    return !outputsSettings.disabled.includes(outputId);
  };
  const isExclusive = (outputId: string) => {
    if (!outputsSettings) return false;
    return outputsSettings.exclusive.includes(outputId);
  };
  const startRename = (outputId: string, currentName: string) => {
    setRenamingOutput(outputId);
    setRenameDraft(currentName);
  };
  const cancelRename = () => {
    setRenamingOutput(null);
    setRenameDraft("");
  };
  const saveRename = (outputId: string) => {
    const value = renameDraft.trim();
    onRenameOutput(outputId, value);
    setRenamingOutput(null);
    setRenameDraft("");
  };

  const providerStatus = (state: string) => {
    const normalized = state.toLowerCase();
    if (normalized === "available" || normalized === "connected" || normalized === "configured" || normalized === "online") {
      return { label: normalized.toUpperCase(), color: "var(--accent-2)" };
    }
    if (normalized === "idle" || normalized === "discovered") {
      return { label: normalized.toUpperCase(), color: "var(--muted-2)" };
    }
    return { label: normalized.toUpperCase(), color: "var(--accent)" };
  };

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
            className={`settings-tab ${isOutputs ? "active" : ""}`}
            onClick={() => onSectionChange("outputs")}
          >
            Outputs
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

        {isOutputs ? (
          <div className="outputs-settings">
            {outputsError ? <div className="muted small">{outputsError}</div> : null}
            {outputsLoading ? <div className="muted small">Loading outputs...</div> : null}
            {outputsProviders.map((providerEntry) => {
              const { provider, outputs, address } = providerEntry;
              const status = providerStatus(provider.state);
              const lastRefresh = outputsLastRefresh[provider.id];
              return (
                <div key={provider.id} className="outputs-provider card">
                  <div className="outputs-provider-header">
                    <div className="outputs-provider-title">
                      <div className="outputs-provider-name">{provider.name}</div>
                      {provider.kind === "bridge" && address ? (
                        <div className="outputs-provider-address muted small">{address}</div>
                      ) : null}
                      <div
                        className="outputs-provider-status"
                        style={{ borderColor: status.color, color: status.color }}
                      >
                        <Circle className="status-icon" fill={status.color} />
                        {status.label}
                      </div>
                      {lastRefresh ? (
                        <span className="outputs-provider-updated muted small">
                          Updated {lastRefresh}
                        </span>
                      ) : null}
                    </div>
                    <button
                      className="btn ghost small outputs-refresh"
                      onClick={() => onRefreshProvider(provider.id)}
                    >
                      <RefreshCw className="icon" />
                      Refresh
                    </button>
                  </div>
                  <div className="outputs-provider-body">
                    {outputs.length === 0 ? (
                      <div className="outputs-empty muted small">
                        No devices found. Refresh to scan again.
                      </div>
                    ) : (
                      outputs.map((output) => {
                        const enabled = isEnabled(output.id);
                        const displayName = resolvedName(output.id, output.name);
                        const isRenaming = renamingOutput === output.id;
                        return (
                          <div key={output.id} className="outputs-device-row">
                            <div className="outputs-device-meta">
                              {isRenaming ? (
                                <input
                                  className="settings-input outputs-rename-input"
                                  value={renameDraft}
                                  onChange={(event) => setRenameDraft(event.target.value)}
                                  onKeyDown={(event) => {
                                    if (event.key === "Enter") saveRename(output.id);
                                    if (event.key === "Escape") cancelRename();
                                  }}
                                  onBlur={() => saveRename(output.id)}
                                  autoFocus
                                />
                              ) : (
                                <div className={`outputs-device-name ${enabled ? "" : "muted"}`}>
                                  {displayName}
                                  <button
                                    className="icon-btn outputs-rename-btn"
                                    onClick={() => startRename(output.id, displayName)}
                                    aria-label="Rename device"
                                  >
                                    <Edit2 className="icon" />
                                  </button>
                                </div>
                              )}
                            </div>
                            <div className="outputs-device-actions">
                              <label className="outputs-toggle">
                                <input
                                  type="checkbox"
                                  checked={enabled}
                                  onChange={(event) => onToggleOutput(output.id, event.target.checked)}
                                />
                                <span className="outputs-toggle-track" />
                                <span className="outputs-toggle-thumb" />
                              </label>
                              {provider.kind === "bridge" ? (
                                <label className="outputs-toggle-group">
                                  <span className="outputs-toggle-label">Exclusive</span>
                                  <span className="outputs-toggle">
                                    <input
                                      type="checkbox"
                                      checked={isExclusive(output.id)}
                                      onChange={(event) => onToggleExclusive(output.id, event.target.checked)}
                                    />
                                    <span className="outputs-toggle-track" />
                                    <span className="outputs-toggle-thumb" />
                                  </span>
                                </label>
                              ) : null}
                            </div>
                          </div>
                        );
                      })
                    )}
                  </div>
                </div>
              );
            })}
            {!outputsLoading && outputsProviders.length === 0 ? (
              <div className="muted small">No providers available.</div>
            ) : null}
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
