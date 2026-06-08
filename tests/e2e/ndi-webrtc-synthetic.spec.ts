import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// REQUIRED real-frame NDI→WebRTC test (the regression guard for the #336
// "connected but black screen" bug).
//
// Unlike the capability-gated tests in ndi-webrtc.spec.ts, this test does NOT
// skip — it asserts that actual H264 frames decode in a real browser. It is
// driven by the `e2e-ndi` self-hosted CI lane, which:
//   1. Starts the synthetic NDI sender (`ndi_test_sender`, publishes
//      "<host> (PRESENTER-TEST)") BEFORE Playwright runs, and
//   2. Runs ONLY this file (`--grep "@synthetic-ndi"`).
// The default ubuntu `e2e` job EXCLUDES it (`--grep-invert "@synthetic-ndi"`)
// because that runner has no NDI SDK / GPU encoder.
//
// Tags: @video-codec routes it to the real-Chrome (H.264) Playwright project;
// @synthetic-ndi selects it into the self-hosted lane.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("NDI video decodes real frames end-to-end (synthetic source) @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  // Discover the synthetic NDI source the lane published. The machine-name
  // prefix varies per host, so match on the "(PRESENTER-TEST)" suffix.
  // NDI discovery on a freshly-started server takes a few seconds, so poll
  // (up to ~30s) rather than querying once.
  let synthetic: { name: string } | undefined;
  for (let i = 0; i < 30; i++) {
    const resp = await request.get(new URL("/ndi/sources", baseURL).toString());
    if (resp.ok()) {
      const list = await resp.json();
      if (Array.isArray(list)) {
        synthetic = list.find((s: { name: string }) =>
          s.name.includes("(PRESENTER-TEST)"),
        );
        if (synthetic) break;
      }
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  // NOT a skip: on the e2e-ndi lane the synthetic sender MUST be running. If
  // it isn't, that is a real failure (broken lane), per test-strictness.
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();

  // Clean slate, then create + activate the synthetic source.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Synthetic-E2E", ndiName: synthetic!.name } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();
  expect(
    (
      await request.post(
        new URL(
          `/integrations/video-sources/${src.id}/activate`,
          baseURL,
        ).toString(),
        { data: {} },
      )
    ).status(),
  ).toBe(200);

  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  // Collect console errors/warnings — a working stream must be clean.
  // ALLOWED: the benign `/stage/snapshot` 404 that fires on load when no
  // presentation is active (pre-existing, unrelated to NDI; the client handles
  // it gracefully). Everything else — including WHEP/ICE errors and watchdog
  // stall warnings — must be absent once video is flowing.
  const ALLOWED = [/\/stage\/snapshot.*404/i, /Failed to load resource.*404.*snapshot/i];
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED.some((re) => re.test(text))) {
        consoleMessages.push(`[${msg.type()}] ${text}`);
      }
    }
  });

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // The core assertion: the <video> must DECODE real frames. The #336
  // regression left videoWidth=0 / readyState=0 / currentTime=0 forever
  // (ICE never connected, then DTLS hung, then PT mismatch, then no media).
  const flowing = await page
    .locator('[data-role="ndi-video"]')
    .evaluate(async (el: HTMLVideoElement) => {
      for (let i = 0; i < 150; i++) {
        if (el.videoWidth > 0 && el.readyState >= 2 && el.currentTime > 0.2) {
          return {
            ok: true,
            videoWidth: el.videoWidth,
            videoHeight: el.videoHeight,
            readyState: el.readyState,
            currentTime: el.currentTime,
          };
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      return {
        ok: false,
        videoWidth: el.videoWidth,
        videoHeight: el.videoHeight,
        readyState: el.readyState,
        currentTime: el.currentTime,
      };
    });

  expect(
    flowing.ok,
    `NDI video must decode frames — got videoWidth=${flowing.videoWidth}, ` +
      `readyState=${flowing.readyState}, currentTime=${flowing.currentTime}`,
  ).toBe(true);
  expect(flowing.videoWidth).toBeGreaterThan(0);
  expect(flowing.readyState).toBeGreaterThanOrEqual(2);

  // currentTime must keep advancing (not a single frozen frame).
  const t1 = flowing.currentTime;
  await page.waitForTimeout(1500);
  const t2 = await page
    .locator('[data-role="ndi-video"]')
    .evaluate((el: HTMLVideoElement) => el.currentTime);
  expect(t2, "video playback must keep advancing").toBeGreaterThan(t1);

  // Clean console — no WHEP/ICE errors once the stream is healthy.
  expect(consoleMessages, `console must be clean: ${consoleMessages.join("; ")}`).toEqual(
    [],
  );

  // Cleanup.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});
