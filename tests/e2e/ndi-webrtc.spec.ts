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
  // Acceptable shapes:
  //   201 — WHEP spec: pipeline ready, returned SDP answer (Location header set)
  //   200 — legacy fallback (kept for defensive compatibility)
  //   204 — configured-but-not-producing source (#431): the source is in the DB
  //         but has no active pipeline (real NDI absent in CI), so the shim
  //         returns 204 No Content, NOT 404 — the client treats it as a quiet
  //         "not producing yet" state with no browser console error.
  //   503 — pipeline starting / source not connected (real NDI absent in CI)
  // 500 / 404 / 4xx-other are bugs (404 is the #431 regression and is banned).
  expect([200, 201, 204, 503]).toContain(whep.status());
  expect(whep.status()).not.toBe(404);
  if (whep.status() === 200 || whep.status() === 201) {
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
// Regression test for the production autoplay-policy bug surfaced by the
// user on 2026-05-20: <video> mounted via DOM mutation with srcObject set
// programmatically ended up `paused=true` in real Chrome on Windows, even
// with `autoplay muted playsinline` attributes. The user saw a fully black
// screen until they right-clicked the (hidden) video element, enabled
// controls, and pressed Play manually.
//
// This bug was INVISIBLE to E2E because Playwright launches Chromium with
// autoplay restrictions DISABLED by default. The Playwright config has been
// updated to launch with `--autoplay-policy=user-gesture-required` so this
// test (and any future test asserting playback) reproduces real Chrome
// behaviour. Without the `video.play()` call in `ndi_video.rs` that
// follows the `set_src_object()` call, this test fails: the video element
// stays `paused=true`, `currentTime=0` indefinitely.
// ─────────────────────────────────────────────────────────────────────────
// `@video-codec` tag routes this test into the `chrome-video` Playwright
// project (real Chrome with H.264 + autoplay policy enforced) per
// playwright.config.ts. Without the tag the test runs against default
// Chromium which can't decode H.264 — the assertion would fail for the
// wrong reason and the autoplay regression would still slip past CI.
test("NdiVideo actually starts playing (autoplay policy regression) @video-codec", async ({ page }) => {
  const status = await page.request.get(new URL("/ndi/status", baseURL).toString());
  const { available } = await status.json();
  test.skip(!available, "NDI SDK not available on this host");

  const sourcesResp = await page.request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = sourcesResp.ok() ? await sourcesResp.json() : [];
  test.skip(
    !Array.isArray(sources) || sources.length === 0,
    "No NDI sources on network — autoplay regression test can't be exercised",
  );
  const ndiName = sources[0].name;

  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Autoplay-Regression", ndiName } },
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

  // Poll for actual playback. videoWidth > 0 alone isn't enough — a paused
  // video can have videoWidth set after metadata loads. The autoplay bug
  // surfaces as `paused=true` AND `currentTime=0` (never advances). Assert
  // BOTH have changed within a generous timeout.
  const playback = await page
    .locator('[data-role="ndi-video"]')
    .evaluate(async (el: HTMLVideoElement) => {
      for (let i = 0; i < 100; i++) {
        if (!el.paused && el.currentTime > 0.1) {
          return {
            ok: true,
            paused: el.paused,
            currentTime: el.currentTime,
            videoWidth: el.videoWidth,
          };
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      return {
        ok: false,
        paused: el.paused,
        currentTime: el.currentTime,
        videoWidth: el.videoWidth,
      };
    });
  expect(
    playback.ok,
    `video failed to start playing — paused=${playback.paused}, currentTime=${playback.currentTime}, videoWidth=${playback.videoWidth}`,
  ).toBe(true);
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
  // On CI runners the NDI SDK isn't loaded and /ndi/sources returns 503 with
  // an error-shaped JSON body, not an array — guard on shape, not just length.
  const sources = sourcesResp.ok() ? await sourcesResp.json() : [];
  test.skip(
    !Array.isArray(sources) || sources.length === 0,
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

// ─────────────────────────────────────────────────────────────────────────
// Hardening: end-to-end "I just opened the URL and video is flowing" tests.
//
// These exist because earlier I claimed the pipeline worked on dev but
// re-navigating to /stage 10 minutes later gave a black screen — pipeline
// drift between consumer sessions. The tests below force the same flow a
// church operator would do: activate a source, open the URL fresh, expect
// video. Then open the URL again, expect video again. Then open it a third
// time. If ANY of those navigations fails to render frames, the test fails.
// ─────────────────────────────────────────────────────────────────────────

test("video flows on fresh /stage navigation after activate (requires live NDI)", async ({
  page,
  request,
}) => {
  // Capability gate.
  const sourcesResp = await request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = sourcesResp.ok() ? await sourcesResp.json() : [];
  test.skip(
    !Array.isArray(sources) || sources.length === 0,
    "No NDI sources on network — first-navigation video flow can't be tested",
  );
  const ndiName = sources[0].name;

  // Clean slate every run.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );

  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "First-Nav-Video-Flow", ndiName } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  const activate = await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  expect(activate.status()).toBe(200);

  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  // Single fresh navigation — operator hitting /stage in a browser.
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });

  // The <video> element should mount AND its videoWidth should resolve > 0
  // within 10s of mount (WHEP signalling, ICE handshake, first frame).
  const flowing = await page.locator('[data-role="ndi-video"]').evaluate(
    async (el: HTMLVideoElement) => {
      for (let i = 0; i < 100; i++) {
        if (el.videoWidth > 0 && el.readyState >= 2) {
          return { videoWidth: el.videoWidth, videoHeight: el.videoHeight, readyState: el.readyState };
        }
        await new Promise((r) => setTimeout(r, 100));
      }
      return { videoWidth: el.videoWidth, videoHeight: el.videoHeight, readyState: el.readyState };
    },
  );
  expect(flowing.videoWidth, "videoWidth must be > 0 on fresh navigation").toBeGreaterThan(0);
  expect(flowing.readyState, "readyState must be HAVE_CURRENT_DATA or better").toBeGreaterThanOrEqual(2);

  // Cleanup.
  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});

test("video keeps flowing across multiple fresh navigations (requires live NDI)", async ({
  page,
  context,
  request,
}) => {
  // The fragility I kept hitting: first /stage navigation worked, second
  // gave black screen because webrtcsink's internal codec discovery state
  // drifted between consumer reconnects. This test loads /stage three
  // times in a row in fresh page contexts and asserts video flows on
  // EVERY load. With the rtpgccbwe (congestion control) element registered
  // statically via gst-plugin-rtp, webrtcsink's state stays stable.

  const sourcesResp = await request.get(
    new URL("/ndi/sources", baseURL).toString(),
  );
  const sources = sourcesResp.ok() ? await sourcesResp.json() : [];
  test.skip(
    !Array.isArray(sources) || sources.length === 0,
    "No NDI sources on network — multi-nav video flow can't be exercised",
  );
  const ndiName = sources[0].name;

  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "Multi-Nav-Test", ndiName } },
  );
  const src = await created.json();
  expect(
    (await request.post(
      new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
      { data: {} },
    )).status(),
  ).toBe(200);
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  // Three sequential page loads — each in a fresh context.
  for (let nav = 1; nav <= 3; nav++) {
    const navPage = await context.newPage();
    await navPage.goto(new URL(`/stage?nav=${nav}`, baseURL).toString());
    await navPage.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
    await navPage.waitForSelector('body[data-layout-code="ndi-fullscreen"]', { timeout: 10_000 });
    const result = await navPage.locator('[data-role="ndi-video"]').evaluate(
      async (el: HTMLVideoElement) => {
        for (let i = 0; i < 120; i++) {
          if (el.videoWidth > 0 && el.readyState >= 2) {
            return { videoWidth: el.videoWidth, readyState: el.readyState };
          }
          await new Promise((r) => setTimeout(r, 100));
        }
        return { videoWidth: el.videoWidth, readyState: el.readyState };
      },
    );
    expect(
      result.videoWidth,
      `nav ${nav}: videoWidth must be > 0 (got ${result.videoWidth})`,
    ).toBeGreaterThan(0);
    await navPage.close();
  }
  // ignore unused param: page is from the fixture but each iteration uses
  // a freshly-created `navPage` from `context.newPage()`.
  void page;

  await request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
  );
  await request.delete(
    new URL(`/integrations/video-sources/${src.id}`, baseURL).toString(),
  );
});
