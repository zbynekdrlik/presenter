import { test, expect, type BrowserContext, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #479 — the stage shows VIDEO latency (decode→render) as a SEPARATE readout
// next to the web/connection latency. The connection readout ("CONNECTED · N
// ms") is the WS heartbeat round-trip; the new readout ("video · N ms") is the
// stage-side received→displayed lag derived per-frame from rVFC metadata by
// `NdiVideo`'s frame observer.
//
// The real per-frame value needs a live NDI/WebRTC stream (the self-hosted
// `@synthetic-ndi` GPU lane). This deterministic test runs on the standard
// GitHub-hosted `e2e` lane: it drives the readout via the stage test hook
// (`__presenterStageSetVideoLatency`) — the same signal the rVFC observer
// writes — and asserts BOTH readouts render, are distinct, and that clearing
// the value hides the video readout. The derivation math itself
// (rVFC metadata → ms) is unit-tested in `ndi_frame_stats.rs`.
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

/** Open the stage on a video (NDI fullscreen) layout, ready + connected. */
async function openVideoStage(context: BrowserContext): Promise<Page> {
  await context.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "ndi-fullscreen" },
  });
  const stagePage = await context.newPage();
  await stagePage.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await stagePage.waitForFunction(
    () =>
      (window as unknown as { __presenterStageConnectionState?: string })
        .__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
  return stagePage;
}

/** Drive the stage-side video-latency readout (the same signal the rVFC
 * observer writes). `null` clears it. */
async function setVideoLatency(page: Page, ms: number | null): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetVideoLatency?: (v: number | null) => void;
      }
    ).__presenterStageSetVideoLatency?.(value);
  }, ms);
}

test("stage shows video latency as a separate readout next to connection latency", async ({
  context,
}) => {
  const consoleMessages: string[] = [];
  const stagePage = await openVideoStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const connectionEl = stagePage.locator(".stage__connection");
  const videoEl = stagePage.locator(".stage__video-latency");

  // The connection (WS round-trip) readout is always present.
  await expect(connectionEl).toBeVisible();
  await expect(connectionEl).toContainText("CONNECTED");

  // No video flowing yet → the video readout is absent (not just empty).
  await expect(videoEl).toHaveCount(0);

  // A frame's derived latency arrives → the SEPARATE "video · N ms" readout
  // appears with the expected "<number> ms" format.
  await setVideoLatency(stagePage, 42);
  await expect(videoEl).toBeVisible();
  await expect(videoEl).toContainText(/video\s*·\s*42\s*ms/);

  // The two readouts coexist as DISTINCT elements (the user's decision: video
  // latency shown SEPARATELY from connection latency, not combined).
  await expect(connectionEl).toContainText("CONNECTED");
  await expect(connectionEl).not.toContainText("video");
  await expect(videoEl).not.toContainText("CONNECTED");

  // The value updates live (a later, larger figure).
  await setVideoLatency(stagePage, 137);
  await expect(videoEl).toContainText(/video\s*·\s*137\s*ms/);

  // Video stops (source deactivated) → the readout disappears; the connection
  // readout remains.
  await setVideoLatency(stagePage, null);
  await expect(videoEl).toHaveCount(0);
  await expect(connectionEl).toBeVisible();

  // browser-console-zero-errors: no errors/warnings the whole time.
  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});
