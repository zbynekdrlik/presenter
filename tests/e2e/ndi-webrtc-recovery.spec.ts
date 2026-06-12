// SPDX-License-Identifier: MIT
//
// Recovery regression: after the server-side pipeline is forcefully killed
// (simulating an ndisrc "Internal data stream error"), the stage display
// MUST resume playing video WITHOUT a page refresh, within 10 seconds.
//
// Tagged @video-codec so playwright.config.ts routes it through real Chrome
// (channel: "chrome" + --autoplay-policy=user-gesture-required) — the
// default Chromium build has no H264 codec and silently bypasses autoplay.
//
// On CI without a real NDI source, the test is honest: it asserts that the
// `/ndi/sources` endpoint exists and returns a result; if there are no NDI
// sources discoverable, the test skips with a clear reason (NOT a silent
// pass). The recovery logic itself runs unconditionally when preconditions
// are met.

import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  waitForNdiLitePage,
  type ServerHandle,
} from "./support";

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

test("NDI WebRTC recovery @video-codec — video resumes within 10s after server pipeline kill", async ({
  page,
  request,
}) => {
  const consoleErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  });

  // Discover an NDI source on the LAN. On CI without libndi or a real
  // broadcaster, this returns 503 or an empty list; we skip honestly.
  const sourcesResp = await request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  if (!sourcesResp.ok()) {
    test.skip(true, "NDI SDK not available on this runner (/ndi/sources not OK)");
    return;
  }
  const discovered = await sourcesResp.json();
  if (!Array.isArray(discovered) || discovered.length === 0) {
    test.skip(true, "no NDI sources discovered on this runner");
    return;
  }

  // Create + activate a video_source pointing at the first discovered NDI broadcaster.
  const ndiName = discovered[0].name as string;
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "recovery-test", ndiName } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  const activate = await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  if (!activate.ok()) {
    test.skip(
      true,
      `could not activate NDI source (status=${activate.status()}); broadcaster likely unavailable`,
    );
    return;
  }

  // Switch the stage layout to ndi-fullscreen, then open the stage display.
  // EXPERIMENT (#379): the ndi layout serves the lite plain-JS player at
  // /stage/lite; its watchdog (ICE-loss + 10s frame-stall) must recover the
  // stream without a reload, same contract as the WASM client before it.
  const layoutResp = await request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );
  expect(layoutResp.ok(), "switching stage layout to ndi-fullscreen must succeed").toBe(true);

  await page.goto(new URL("/stage", baseURL).toString());
  await waitForNdiLitePage(page);

  const video = page.locator('video[data-role="ndi-video"]').first();
  await expect(video).toBeVisible();

  // Phase 1: initial stream must reach videoWidth > 0.
  await expect
    .poll(async () => await video.evaluate((v: HTMLVideoElement) => v.videoWidth), {
      timeout: 15_000,
      intervals: [500, 500, 1000, 1000, 2000],
      message: "initial NDI video stream never reached videoWidth > 0",
    })
    .toBeGreaterThan(0);

  const beforeKill = await video.evaluate((v: HTMLVideoElement) => ({
    width: v.videoWidth,
    currentTime: v.currentTime,
  }));

  // Phase 2: kill the server-side pipeline (simulates ndisrc crash).
  // Requires the `test-helpers` cargo feature — the route is absent in prod builds.
  const killResp = await request.post(
    new URL(`/test/ndi/kill-pipeline/${src.id}`, baseURL).toString(),
  );
  if (killResp.status() === 404 && (await killResp.text()).includes("Not Found")) {
    test.skip(
      true,
      "binary built without `test-helpers` feature; the kill route is absent",
    );
    return;
  }
  expect(killResp.status(), "kill endpoint must return 204").toBe(204);

  // Phase 3: within 10s, the browser must reconnect AND new frames must be
  // decoded. The poll predicate combines BOTH conditions so a partial recovery
  // (videoWidth flickers but currentTime stuck) keeps polling rather than
  // exiting falsely-green.
  await expect
    .poll(
      async () =>
        await video.evaluate(
          ({ beforeKillTime }: { beforeKillTime: number }) =>
            (() => {
              const v = document.querySelector(
                'video[data-role="ndi-video"]',
              ) as HTMLVideoElement | null;
              if (!v) return false;
              return v.videoWidth > 0 && v.currentTime > beforeKillTime + 0.1;
            })(),
          { beforeKillTime: beforeKill.currentTime },
        ),
      {
        timeout: 10_000,
        intervals: [500, 500, 1000, 1000, 2000, 2000],
        message:
          "video did not recover within 10s after server pipeline kill " +
          "(videoWidth > 0 AND currentTime advanced past beforeKill)",
      },
    )
    .toBe(true);

  // No console errors during the whole recovery cycle.
  expect(consoleErrors).toEqual([]);
});
