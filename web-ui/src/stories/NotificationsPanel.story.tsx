import type { ToastNotification } from "../hooks/useToasts";
import NotificationsPanel from "../components/NotificationsPanel";
import { action } from "storybook/actions";
import "../styles.css";

const notifications: ToastNotification[] = [
  {
    id: 1,
    level: "info",
    message: "Library scan started.",
    createdAt: new Date("2026-03-02T10:05:00Z")
  },
  {
    id: 2,
    level: "warn",
    message: "Output lock is held by another session.",
    createdAt: new Date("2026-03-02T10:06:00Z")
  },
  {
    id: 3,
    level: "error",
    message: "Status stream disconnected.",
    createdAt: new Date("2026-03-02T10:07:00Z")
  }
];

export default {
  title: "Layout/NotificationsPanel",
  component: NotificationsPanel,
  argTypes: {
    open: { control: "boolean" },
    showGate: { control: "boolean" },
    withItems: { control: "boolean" }
  }
};

type NotificationsPanelArgs = {
  open: boolean;
  showGate: boolean;
  withItems: boolean;
};

const Template = (args: NotificationsPanelArgs) => (
  <div style={{ minHeight: "100vh", background: "#101317" }}>
    <NotificationsPanel
      open={args.open}
      showGate={args.showGate}
      notifications={args.withItems ? notifications : []}
      onClose={action("close")}
      onClear={action("clear")}
    />
  </div>
);

export const WithItems = Template.bind({});
WithItems.args = {
  open: true,
  showGate: false,
  withItems: true
};

export const Empty = Template.bind({});
Empty.args = {
  open: true,
  showGate: false,
  withItems: false
};

export const HiddenByGate = Template.bind({});
HiddenByGate.args = {
  open: true,
  showGate: true,
  withItems: true
};
