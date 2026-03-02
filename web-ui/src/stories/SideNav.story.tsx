import SideNav from "../components/SideNav";
import { action } from "storybook/actions";
import "../styles.css";

export default {
  title: "Layout/SideNav",
  component: SideNav,
  argTypes: {
    navCollapsed: { control: "boolean" },
    settingsOpen: { control: "boolean" }
  }
};

type SideNavArgs = {
  navCollapsed: boolean;
  settingsOpen: boolean;
};

const Template = (args: SideNavArgs) => (
  <div style={{ minHeight: "100vh", display: "grid", gridTemplateColumns: "280px 1fr" }}>
    <SideNav
      navCollapsed={args.navCollapsed}
      settingsOpen={args.settingsOpen}
      onToggleCollapsed={action("toggle-collapsed")}
      navigateTo={action("navigate-to")}
    />
    <main style={{ padding: 24 }} />
  </div>
);

export const Library = Template.bind({});
Library.args = {
  navCollapsed: false,
  settingsOpen: false
};

export const Settings = Template.bind({});
Settings.args = {
  navCollapsed: false,
  settingsOpen: true
};

export const Collapsed = Template.bind({});
Collapsed.args = {
  navCollapsed: true,
  settingsOpen: false
};
