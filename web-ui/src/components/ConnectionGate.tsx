import { useEffect, useState } from "react";

interface ConnectionGateProps {
  status: "connecting" | "disconnected";
  message: string | null;
  apiBase: string;
  apiBaseDefault: string;
  onApiBaseChange: (value: string) => void;
  onApiBaseReset: () => void;
  onReconnect: () => void;
}

export default function ConnectionGate({
  status,
  message,
  apiBase,
  apiBaseDefault,
  onApiBaseChange,
  onApiBaseReset,
  onReconnect
}: ConnectionGateProps) {
  const [apiBaseDraft, setApiBaseDraft] = useState(apiBase);

  useEffect(() => {
    setApiBaseDraft(apiBase);
  }, [apiBase]);

  const trimmedDraft = apiBaseDraft.trim();
  const isDirty = trimmedDraft !== apiBase.trim();
  const effectiveBase = apiBase.trim() || apiBaseDefault.trim();

  return (
    <div className="connection-gate">
      <div className="connection-card">
        <div className="connection-eyebrow">Audio Hub</div>
        <h1>{status === "connecting" ? "Connecting" : "Server offline"}</h1>
        <p className="connection-subtitle">
          {status === "connecting"
            ? "Waiting for the hub server to respond."
            : "Set the hub server address and reconnect."}
        </p>
        {message ? <div className="connection-alert">{message}</div> : null}
        <div className="connection-form">
          <label className="settings-label" htmlFor="connection-api-base">
            API base URL
          </label>
          <input
            id="connection-api-base"
            className="settings-input"
            type="url"
            placeholder={apiBaseDefault || "http://<SERVER_IP>:8080"}
            value={apiBaseDraft}
            onChange={(event) => setApiBaseDraft(event.target.value)}
          />
          <div className="connection-actions">
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
            <button className="btn ghost small" onClick={onReconnect}>
              Reconnect
            </button>
            <button
              className="btn solid small"
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
    </div>
  );
}
