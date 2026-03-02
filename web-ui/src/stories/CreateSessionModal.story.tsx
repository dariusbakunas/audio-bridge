import CreateSessionModal from "../components/CreateSessionModal";
import { action } from "storybook/actions";
import "../styles.css";

export default {
  title: "Sessions/CreateSessionModal",
  component: CreateSessionModal,
  argTypes: {
    open: { control: "boolean" },
    busy: { control: "boolean" },
    name: { control: "text" },
    neverExpires: { control: "boolean" }
  }
};

type CreateSessionModalArgs = {
  open: boolean;
  busy: boolean;
  name: string;
  neverExpires: boolean;
};

const Template = (args: CreateSessionModalArgs) => (
  <CreateSessionModal
    open={args.open}
    busy={args.busy}
    name={args.name}
    neverExpires={args.neverExpires}
    onNameChange={action("name-change")}
    onNeverExpiresChange={action("never-expires-change")}
    onClose={action("close")}
    onSubmit={action("submit")}
  />
);

export const Default = Template.bind({});
Default.args = {
  open: true,
  busy: false,
  name: "Session 3",
  neverExpires: false
};

export const NeverExpires = Template.bind({});
NeverExpires.args = {
  open: true,
  busy: false,
  name: "Kitchen Session",
  neverExpires: true
};

export const Busy = Template.bind({});
Busy.args = {
  open: true,
  busy: true,
  name: "Creating...",
  neverExpires: false
};
