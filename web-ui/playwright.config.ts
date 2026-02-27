import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "on-first-retry"
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] }
    }
  ],
  webServer: [
    {
      command:
        "cd .. && cargo run -p audio-hub-server -- --config web-ui/tests/fixtures/e2e.server.toml --bind 127.0.0.1:18080 --media-dir web-ui/tests/fixtures/media --metadata-db-path /tmp/audio-hub-e2e.sqlite",
      url: "http://127.0.0.1:18080/health",
      reuseExistingServer: !process.env.CI,
      timeout: 180000
    },
    {
      command: "VITE_API_BASE=http://127.0.0.1:18080 npm run dev -- --host 127.0.0.1 --port 5173",
      url: "http://127.0.0.1:5173",
      reuseExistingServer: !process.env.CI,
      timeout: 120000
    }
  ]
});
