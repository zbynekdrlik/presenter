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
        // Two non-default settings — BOTH are required to catch real-world
        // video playback bugs in CI. Neither was in place before, and both
        // were proven necessary on 2026-05-20 when a user reported a fully
        // black `<video>` element that Playwright's default config had
        // silently passed for weeks.
        //
        // 1. `channel: "chrome"` — use the branded Google Chrome binary
        //    (with proprietary codecs including H.264) rather than the
        //    default open-source Chromium. Default Chromium lacks H.264
        //    by license; WebRTC video tracks with H.264 RTP payloads will
        //    have their `ontrack` callback fire and even set `srcObject`,
        //    but the actual frame decode silently fails — `videoWidth`
        //    stays 0, `currentTime` never advances, and a `paused=true`
        //    assertion looks no different from "the bug we're trying to
        //    catch". This is the difference Eyevinn and other WebRTC
        //    teams documented years ago but it's not on the Playwright
        //    quickstart page, so it's still a routine trap.
        //    Requires `npx playwright install chrome` in CI setup (see
        //    .github/workflows/*.yml).
        //
        // 2. `--autoplay-policy=user-gesture-required` — match real Chrome
        //    behaviour for autoplay. Default Playwright launches with the
        //    policy DISABLED, which means programmatic `srcObject` +
        //    `autoplay muted playsinline` element silently auto-played in
        //    every CI test while the same code paused in real Chrome on
        //    every user's machine. With the policy enforced, the test
        //    will FAIL if `video.play()` is not called explicitly after
        //    setting `srcObject`.
        channel: "chrome",
        launchOptions: {
          args: ["--autoplay-policy=user-gesture-required"],
        },
      },
    },
  ],
});
