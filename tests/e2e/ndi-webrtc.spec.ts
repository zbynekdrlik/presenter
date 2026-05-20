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
  // On hosts without a live NDI source (CI runners with no libndi/VA-API)
  // the WHEP POST returns 503 because the pipeline can't start. Those
  // errors are expected here — this test only asserts DOM structure.
  const ALLOWED = [
    /Failed to load resource.*503/i,
    /WHEP connect for.*failed/i,
    /WHEP POST returned 503/i,
  ];
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED.some((re) => re.test(text))) {
        consoleMessages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  page.on("pageerror", (err) => {
    if (!ALLOWED.some((re) => re.test(err.message))) {
      consoleMessages.push(`[pageerror] ${err.message}`);
    }
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

// ─────────────────────────────────────────────────────────────────────────
// Regression tests for the "Connecting…" overlay state machine.
//
// The bug they catch (manually surfaced 2026-05-19): the WS event
// NdiConnectionStatus sets ctx.ndi_status="connecting" when the source is
// activated, and the stage overlay renders "Connecting…" while that status
// holds. The overlay is supposed to clear once the pipeline reaches Streaming
// AND show the actual error if pipeline build fails. Without these tests,
// either failure mode (video plays under a stuck "Connecting…" overlay, OR
// activate errors with no operator-visible feedback) goes unnoticed.
// ─────────────────────────────────────────────────────────────────────────

test("stage clears Connecting overlay when activate succeeds (requires live NDI)", async ({ page }) => {
  // Capability gate — needs a real NDI broadcaster reachable on the LAN.
  // On CI the network is empty; skip there so this remains green on cold runners
  // but exercises the success path when a developer runs against dev.
  const sourcesResp = await page.request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = await sourcesResp.json();
  test.skip(
    sources.length === 0,
    "No NDI sources on network — overlay-clear path can't be exercised",
  );

  // Pick the first discovered source as the broadcaster.
  const ndiName = sources[0].name;
  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Overlay-Clear-Test", ndiName } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  // activate is allowed to take a few seconds — start_pipeline blocks until
  // webrtcsink's video pad has negotiated caps, then publishes
  // NdiConnectionStatus="connected" via the live hub.
  const activate = await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  expect(activate.status()).toBe(200);

  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', { timeout: 10_000 });

  // The WS event NdiConnectionStatus="connected" should arrive shortly after
  // the page opens. Once it does, the <Show> guard hides .stage-ndi__overlay.
  // We wait up to 10s — well above the typical end-to-end overlay-clear time
  // (~1s on dev2) but far below what a hang would take.
  await page.waitForFunction(
    () => !document.querySelector(".stage-ndi__overlay"),
    null,
    { timeout: 10_000 },
  );

  // Sanity: the <NdiVideo> mounted with the right source id, AND no MJPEG
  // leftover. (Catches a regression where the overlay vanishes for the wrong
  // reason — e.g. the layout broke entirely.)
  const video = page.locator('[data-role="ndi-video"]');
  await expect(video).toHaveCount(1);
  await expect(video).toHaveAttribute("data-source-id", src.id);
  await expect(page.locator('.stage-ndi__overlay')).toHaveCount(0);

  // Cleanup.
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await page.request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});

// The failure-path overlay coverage is a Rust unit test in
// `crates/presenter-ui/src/components/stage/mod.rs::tests` — exercising the
// `ndi_status_text` mapping directly catches a regression to the
// status→overlay-text logic without needing a live server + bogus NDI source
// (which crashed the spawned test server in CI because ndisrc retries forever
// before the start_pipeline timeout fires). The pure-function unit test runs
// fast on any host, has no GStreamer/libnice/libndi dependency, and covers
// the exact bug surface: status="failed: …" must render "NDI pipeline failed: …".
