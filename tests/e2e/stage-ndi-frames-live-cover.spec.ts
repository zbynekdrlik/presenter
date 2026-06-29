import { test, expect, type BrowserContext, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #500 — the ndi-fullscreen NEUTRAL covering placeholder must reflect whether
// frames are ACTUALLY presenting, not just the lagging server status.
//
// Live on prod 2026-06-29 (v0.4.170), a late-joining stage client (the operator
// header preview iframe) held `ndi_status="connecting"` for up to ~30s — until
// the server's next NDI-status tick — while the WHEP `<video>` was ALREADY
// decoding frames (1280x720, readyState=4, "VIDEO · 52 MS"). The gray
// "Connecting…" cover hid that live video for ~30s, so the operator saw "no NDI
// preview". The fix gates the cover on `!ndi_frames_live`: as soon as frames are
// presenting, the cover drops; when frames stop it reappears.
//
// The real per-frame value needs a live NDI/WebRTC stream (the self-hosted GPU
// lane). This deterministic test runs on the standard GitHub-hosted `e2e` lane,
// which has NO NDI SDK — so `activate` succeeds without ever starting a pipeline
// and the client holds the neutral `connecting` state (the exact late-join
// state). It drives the frames-live flag via the stage test hook
// (`__presenterStageSetNdiFramesLive`) — the SAME signal the rVFC observer
// writes per frame — and asserts the cover hides when frames are live and
// reappears when they are not. The pure gate (`should_show_neutral_cover`) and
// the staleness helper are unit-tested in `mod.rs` / `ndi_frame_stats.rs`.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 120_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/** Create + activate a not-producing source (no SDK → activate succeeds and the
 * client holds the neutral `connecting` state), select ndi-fullscreen, and open
 * the stage page ready + connected. Returns the page. */
async function openActiveNdiStage(context: BrowserContext): Promise<Page> {
  const created = await context.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-#500", ndiName: "RESOLUME-SNV (cg-obs)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  const activate = await context.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  expect(activate.status()).toBe(200);

  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });

  const page = await context.newPage();
  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
    timeout: 10_000,
  });
  return page;
}

/** Drive the "frames are presenting" flag (the same signal the rVFC observer
 * writes per frame). */
async function setNdiFramesLive(page: Page, live: boolean): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetNdiFramesLive?: (v: boolean) => void;
      }
    ).__presenterStageSetNdiFramesLive?.(value);
  }, live);
}

test("ndi-fullscreen neutral cover drops when frames are presenting and returns when they stop (#500)", async ({
  context,
}) => {
  // Expected, non-error console on a host with no live NDI source: the WHEP POST
  // is answered 503 (no SDK) / 204 (configured-but-not-producing, #431) and the
  // client backs off quietly. Keep these TIGHT so a genuine WebRTC error is not
  // swallowed (same allow-list as the #448 cover test).
  const ALLOWED = [
    /Failed to load resource.*\b(503|204)\b/i,
    /WHEP (POST|connect)[^\n]*\b(503|204)\b/i,
  ];
  const consoleMessages: string[] = [];
  const collect = (text: string) => {
    if (!ALLOWED.some((re) => re.test(text))) consoleMessages.push(text);
  };

  const page = await openActiveNdiStage(context);
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      collect(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => collect(`[pageerror] ${err.message}`));

  const cover = page.locator(".stage-ndi__placeholder--cover");
  const video = page.locator('[data-role="ndi-video"]');

  // The <NdiVideo> mounts (active source), and with NO frames yet the neutral
  // covering placeholder is shown over the bare <video> (the genuine pre-video
  // state — #448). It carries a calm neutral message, never a red error.
  await expect(video).toHaveCount(1, { timeout: 10_000 });
  await expect(cover).toBeVisible({ timeout: 10_000 });
  await expect(cover).toHaveText(/Waiting for video source|Connecting/i);
  await expect(page.locator(".stage-ndi__overlay")).toHaveCount(0);

  // #500 core: frames start presenting (rVFC would fire) while the server status
  // is STILL the stale `connecting` — the cover must drop immediately so the
  // already-decoding video is visible, NOT hidden for ~30s.
  await setNdiFramesLive(page, true);
  await expect(cover).toHaveCount(0);
  // The live video element remains mounted and is no longer covered.
  await expect(video).toHaveCount(1);
  // Still no red error overlay — the frames gate never suppresses errors, and
  // there is no error here anyway.
  await expect(page.locator(".stage-ndi__overlay")).toHaveCount(0);

  // Frames stop (source went silent / stalled) → the neutral cover returns so a
  // genuinely-dead source is not left showing a frozen last frame with no hint.
  await setNdiFramesLive(page, false);
  await expect(cover).toBeVisible();
  await expect(cover).toHaveText(/Waiting for video source|Connecting/i);

  // And it drops again on the next batch of frames (the steady late-join path).
  await setNdiFramesLive(page, true);
  await expect(cover).toHaveCount(0);

  // browser-console-zero-errors: clean console the whole time.
  expect(consoleMessages).toEqual([]);

  await page.close();
});
