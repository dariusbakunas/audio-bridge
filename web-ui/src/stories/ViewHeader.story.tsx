import type { SessionSummary } from "../types";
import ViewHeader from "../components/ViewHeader";
import { action } from "storybook/actions";
import "../styles.css";

const sessions: SessionSummary[] = [
  {
    id: "sess:default",
    name: "Default",
    mode: "remote",
    client_id: "web-default-global",
    app_version: "0.14.1",
    owner: "web-ui",
    active_output_id: "bridge:living-room:default",
    queue_len: 8,
    created_age_ms: 200000,
    last_seen_age_ms: 500
  },
  {
    id: "sess:local",
    name: "Local",
    mode: "local",
    client_id: "web-client",
    app_version: "0.14.1",
    owner: "web-ui",
    active_output_id: null,
    queue_len: 2,
    created_age_ms: 110000,
    last_seen_age_ms: 250
  }
];

export default {
  title: "Layout/ViewHeader",
  component: ViewHeader,
  argTypes: {
    canGoBack: { control: "boolean" },
    canGoForward: { control: "boolean" },
    viewTitle: { control: "text" },
    showLibraryTools: { control: "boolean" },
    albumSearch: { control: "text" },
    albumViewMode: { control: { type: "inline-radio" }, options: ["grid", "list"] },
    deleteSessionDisabled: { control: "boolean" },
    notificationsOpen: { control: "boolean" },
    unreadCount: { control: { type: "number", min: 0 } },
    serverConnected: { control: "boolean" }
  }
};

type ViewHeaderArgs = {
  canGoBack: boolean;
  canGoForward: boolean;
  viewTitle: string;
  showLibraryTools: boolean;
  albumSearch: string;
  albumViewMode: "grid" | "list";
  deleteSessionDisabled: boolean;
  notificationsOpen: boolean;
  unreadCount: number;
  serverConnected: boolean;
};

const Template = (args: ViewHeaderArgs) => (
  <div style={{ padding: 16 }}>
    <ViewHeader
      canGoBack={args.canGoBack}
      canGoForward={args.canGoForward}
      onGoBack={action("go-back")}
      onGoForward={action("go-forward")}
      viewTitle={args.viewTitle}
      showLibraryTools={args.showLibraryTools}
      albumSearch={args.albumSearch}
      onAlbumSearchChange={action("album-search-change")}
      albumViewMode={args.albumViewMode}
      onAlbumViewModeChange={action("album-view-mode-change")}
      sessionId={sessions[0].id}
      sessions={sessions}
      serverConnected={args.serverConnected}
      onSessionChange={action("session-change")}
      onCreateSession={action("create-session")}
      onDeleteSession={action("delete-session")}
      deleteSessionDisabled={args.deleteSessionDisabled}
      notificationsOpen={args.notificationsOpen}
      unreadCount={args.unreadCount}
      onToggleNotifications={action("toggle-notifications")}
    />
  </div>
);

export const LibraryMode = Template.bind({});
LibraryMode.args = {
  canGoBack: true,
  canGoForward: true,
  viewTitle: "Albums",
  showLibraryTools: true,
  albumSearch: "radiohead",
  albumViewMode: "grid",
  deleteSessionDisabled: false,
  notificationsOpen: false,
  unreadCount: 3,
  serverConnected: true
};

export const SettingsMode = Template.bind({});
SettingsMode.args = {
  canGoBack: true,
  canGoForward: false,
  viewTitle: "Settings",
  showLibraryTools: false,
  albumSearch: "",
  albumViewMode: "list",
  deleteSessionDisabled: true,
  notificationsOpen: false,
  unreadCount: 0,
  serverConnected: true
};

export const Disconnected = Template.bind({});
Disconnected.args = {
  canGoBack: false,
  canGoForward: false,
  viewTitle: "Albums",
  showLibraryTools: true,
  albumSearch: "",
  albumViewMode: "grid",
  deleteSessionDisabled: true,
  notificationsOpen: true,
  unreadCount: 12,
  serverConnected: false
};
