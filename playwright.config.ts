import { defineConfig, devices } from "@playwright/test";

const TEST_PORT = process.env.PRESENTER_PORT ?? "8899";
const TEST_DB_URL = process.env.PRESENTER_DB_URL ?? "sqlite://presenter_e2e.db";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  timeout: 600_000,
  workers: 1,
  expect: {
    timeout: 20_000,
  },
  retries: process.env.CI ? 1 : 0,
  reporter: [["html", { outputFolder: "playwright-report", open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${TEST_PORT}`,
    trace: "on-first-retry",
    browserName: "chromium",
    testIdAttribute: "data-test-id",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
