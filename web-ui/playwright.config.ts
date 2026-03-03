import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  globalSetup: "./tests/e2e/global-setup.ts",
  globalTeardown: "./tests/e2e/global-teardown.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: process.env.E2E_BASE_URL ?? `http://127.0.0.1:${process.env.E2E_HUB_PORT ?? "18080"}`,
    trace: "on-first-retry"
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: process.env.E2E_BASE_URL_CHROMIUM ?? "http://127.0.0.1:18081"
      }
    },
    {
      name: "firefox",
      use: {
        ...devices["Desktop Firefox"],
        baseURL: process.env.E2E_BASE_URL_FIREFOX ?? "http://127.0.0.1:18082"
      }
    },
    {
      name: "webkit",
      use: {
        ...devices["Desktop Safari"],
        baseURL: process.env.E2E_BASE_URL_WEBKIT ?? "http://127.0.0.1:18083"
      }
    }
  ]
});
