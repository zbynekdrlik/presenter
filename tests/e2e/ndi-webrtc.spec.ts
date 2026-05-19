import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
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

test("WHEP endpoint returns SDP answer for active source", async ({ request }) => {
  // Create + activate a source backed by a known NDI source name.
  // The known source is STREAM-SNV (10.77.9.204:5961) on the dev LAN; on CI we
  // accept the fact that no real NDI source exists — the WHEP endpoint must
  // still return 404 (source not active) or 503 (no NDI available), NEVER 500.
  const sources = await request.get(new URL("/integrations/video-sources", baseURL).toString());
  expect(sources.status()).toBe(200);

  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndiName: "STREAM-SNV (stream)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );

  // WHEP POST with a minimal SDP offer body. On a host without a real NDI
  // source we expect the pipeline to enter Starting but never reach Streaming;
  // the WHEP shim must respond with 503 + a body explaining why.
  const offer = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n";
  const whep = await request.post(
    new URL(`/ndi/whep/${src.id}`, baseURL).toString(),
    {
      data: offer,
      headers: { "Content-Type": "application/sdp" },
    },
  );
  // Two acceptable shapes:
  //   200 — pipeline ready, returned SDP answer
  //   503 — pipeline starting / source not connected (real NDI absent in CI)
  // 500 / 404 / 4xx-other are bugs.
  expect([200, 503]).toContain(whep.status());
  if (whep.status() === 200) {
    const answer = await whep.text();
    expect(answer).toMatch(/^v=0/);
    expect(answer).toMatch(/m=video /);
  }
});

test("stage page mounts NdiVideo with correct data attributes when source active", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => {
    consoleMessages.push(`[pageerror] ${err.message}`);
  });

  // Create + activate a source.
  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndiName: "STREAM-SNV (stream)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();
  await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );

  // Switch the stage layout to ndi-fullscreen.
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', { timeout: 10_000 });

  // The new component MUST render exactly one <video data-role="ndi-video"> with
  // data-source-id matching the active source. No <img src="/ndi/mjpeg"> anywhere.
  const videoEl = page.locator('[data-role="ndi-video"]');
  await expect(videoEl).toHaveCount(1);
  await expect(videoEl).toHaveAttribute("data-source-id", src.id);

  // No legacy MJPEG image element should exist anywhere.
  await expect(page.locator('img[src*="/ndi/mjpeg"]')).toHaveCount(0);
  await expect(page.locator('img[src*="/ndi/stream"]')).toHaveCount(0);

  // Browser console must be clean — no errors, no warnings, no page errors.
  expect(consoleMessages).toEqual([]);
});

test("NdiVideo videoWidth resolves above zero within 5 seconds of mount", async ({ page }) => {
  // This test is the actual "video is flowing" check. On CI with no live NDI
  // source it would time out — we mark it skipped when NDI is unavailable.
  const status = await page.request.get(new URL("/ndi/status", baseURL).toString());
  const { available } = await status.json();
  test.skip(!available, "NDI SDK not available on this host");

  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndiName: "STREAM-SNV (stream)" } },
  );
  const src = await created.json();
  await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // Poll videoWidth until > 0 or 5 s timeout.
  const ok = await page
    .locator('[data-role="ndi-video"]')
    .evaluate(
      async (el: HTMLVideoElement) => {
        for (let i = 0; i < 50; i++) {
          if (el.videoWidth > 0) return true;
          await new Promise((r) => setTimeout(r, 100));
        }
        return el.videoWidth > 0;
      },
    );
  expect(ok).toBe(true);
});
