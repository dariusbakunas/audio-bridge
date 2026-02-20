import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  framework: "@storybook/react-vite",
  stories: ["../src/stories/**/*.story.tsx"],
  addons: ["@storybook/addon-docs"]
};

export default config;
