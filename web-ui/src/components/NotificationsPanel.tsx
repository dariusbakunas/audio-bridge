import { ToastNotification } from "../hooks/useToasts";

type NotificationsPanelProps = {
  open: boolean;
  showGate: boolean;
  notifications: ToastNotification[];
  onClose: () => void;
  onClear: () => void;
};

export default function NotificationsPanel({
  open,
  showGate,
  notifications,
  onClose,
  onClear
}: NotificationsPanelProps) {
  if (!open || showGate) return null;

  return (
    <div className="side-panel-backdrop notifications-backdrop" onClick={onClose}>
      <aside
        className="side-panel notification-panel"
        aria-label="Notifications"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="card-header">
          <span>Notifications</span>
          <div className="card-actions">
            <span className="pill">{notifications.length} items</span>
            <button className="btn ghost small" onClick={onClear}>
              Clear
            </button>
            <button className="btn ghost small" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
        <div className="notification-list">
          {notifications.length === 0 ? (
            <div className="muted small">No notifications yet.</div>
          ) : null}
          {notifications.map((entry) => (
            <div key={entry.id} className={`notification-item level-${entry.level}`}>
              <div className="notification-message">{entry.message}</div>
              <div className="notification-time">{entry.createdAt.toLocaleTimeString()}</div>
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
