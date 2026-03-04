import { Plus, Radio, Trash2 } from "lucide-react";

import { SessionSummary } from "../types";

type SessionsScreenProps = {
  sessionId: string | null;
  sessions: SessionSummary[];
  serverConnected: boolean;
  onSessionChange: (nextSessionId: string) => void;
  onCreateSession: () => void;
  onDeleteSession: () => void;
  deleteSessionDisabled: boolean;
};

export default function SessionsScreen({
  sessionId,
  sessions,
  serverConnected,
  onSessionChange,
  onCreateSession,
  onDeleteSession,
  deleteSessionDisabled
}: SessionsScreenProps) {
  return (
    <section className="sessions-screen">
      <div className="sessions-screen-header">
        <h2>Sessions</h2>
        <div className="sessions-screen-actions">
          <button
            className="btn small"
            type="button"
            onClick={onCreateSession}
            disabled={!serverConnected}
            aria-label="Create new session"
          >
            <Plus className="icon" aria-hidden="true" />
            New
          </button>
          <button
            className="btn ghost small"
            type="button"
            onClick={onDeleteSession}
            disabled={deleteSessionDisabled}
            aria-label="Delete selected session"
          >
            <Trash2 className="icon" aria-hidden="true" />
            Delete
          </button>
        </div>
      </div>

      <div className="sessions-screen-list">
        {sessions.map((session) => {
          const active = session.id === sessionId;
          return (
            <button
              key={session.id}
              className={`sessions-screen-card${active ? " active" : ""}`}
              type="button"
              onClick={() => onSessionChange(session.id)}
            >
              <div className="sessions-screen-card-head">
                <div className="sessions-screen-card-title">
                  <Radio className="icon" aria-hidden="true" />
                  <span>{session.name}</span>
                </div>
                {active ? <span className="pill">Active</span> : null}
              </div>
              <div className="sessions-screen-card-subtitle">
                {session.mode === "local" ? "Local playback session" : "Remote playback session"}
              </div>
              <div className="sessions-screen-card-meta">{session.active_output_id ?? "No output selected"}</div>
            </button>
          );
        })}
      </div>
    </section>
  );
}
