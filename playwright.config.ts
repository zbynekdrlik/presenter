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
      // Default project: open-source Chromium (headless shell on CI), used
      // for every test EXCEPT the video-playback tests. The video tests
      // opt into the `chrome-video` project below by adding `@video-codec`
      // to the test title — the `grep`/`grepInvert` filters on each project
      // route tests to one or the other. Forcing every test through branded
      // Chrome made existing "browser console must be clean" assertions
      // fail because real Chrome emits stable-channel telemetry/devtools
      // messages that the open-source Chromium build doesn't.
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
      },
      // Exclude tests tagged @video-codec — those need real Chrome (next
      // project below). Tagging is done in the test title:
      //   test("foo @video-codec", ...)
      grepInvert: /@video-codec/,
    },
    {
      // Real-Chrome project for tests that depend on proprietary codecs
      // (H.264 -> WebRTC video) AND real Chrome autoplay behaviour.
      // Opted into by adding `@video-codec` to the test title — the
      // `grep: /@video-codec/` filter below routes only those tests here.
      // DO NOT make this the default for every test. See ndi-webrtc.spec
      // "NdiVideo actually starts playing (autoplay policy regression)".
      //
      // 1. `channel: "chrome"` — branded Google Chrome binary with H.264.
      //    Default Chromium lacks H.264 by license; WebRTC tracks with
      //    H.264 payloads silently fail to decode in open-source Chromium
      //    (ontrack fires, srcObject set, but videoWidth stays 0). The
      //    open-source Chromium can't be told apart from "real autoplay
      //    bug" using the same assertions, so the test would be useless.
      //    Requires `npx playwright install chrome` in CI setup.
      //
      // 2. `--autoplay-policy=user-gesture-required` — match real Chrome
      //    behaviour. Default Playwright disables the policy entirely,
      //    which means programmatic playback of <video> mounted via DOM
      //    mutation silently auto-played in every CI test while the same
      //    code paused in every real user's Chrome. With the policy on,
      //    the test now FAILS if `video.play()` is missing — which is
      //    the bug surfaced by a user on 2026-05-20 and fixed in 90b30ee.
      name: "chrome-video",
      use: {
        ...devices["Desktop Chrome"],
        channel: "chrome",
        launchOptions: {
          args: ["--autoplay-policy=user-gesture-required"],
        },
      },
      // Only run tests tagged @video-codec — the rest stay in the default
      // chromium project for speed + console-clean assertions.
      grep: /@video-codec/,
    },
  ],
});
