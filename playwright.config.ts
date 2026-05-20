import { defineConfig, devices } from "@playwright/test";

const TEST_PORT = process.env.PRESENTER_PORT ?? "8899";
const TEST_DB_URL = process.env.PRESENTER_DB_URL ?? "sqlite://presenter_e2e.db";

export default defineConfig({
  testDir: "./tests/e2e",
  fullyParallel: false,
  timeout: 180_000, // 3 minutes per test
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
      use: {
        ...devices["Desktop Chrome"],
        // Match real-Chrome behavior: autoplay requires user gesture.
        // Default Playwright launches with autoplay restrictions DISABLED,
        // which silently masked a real production bug — `<video>` element
        // mounted via DOM mutation with `srcObject` set programmatically
        // ended up paused in real Chrome, but Playwright auto-played it.
        // Without this override, no Playwright E2E can catch broken
        // autoplay behavior — the test would always pass while real users
        // saw a black, paused video. See:
        // https://chromium.googlesource.com/chromium/src/+/refs/heads/main/docs/website/site/audio-video/autoplay/index.md
        launchOptions: {
          args: ["--autoplay-policy=user-gesture-required"],
        },
      },
    },
  ],
});
