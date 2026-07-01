import { test, expect, type BrowserContext, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #512 — the stage shows the TRUE server→display video latency as a SEPARATE
// readout next to the web/connection latency. The connection readout
// ("CONNECTED · N ms") is the WS heartbeat round-trip; the video readout
// ("server→displej · N ms") is the network transit (RTT/2 via /ndi/time) plus
// the per-frame render residual (buffer+decode+present) — written per frame by
// `NdiVideo`'s rVFC observer.
//
// The readout is shown whenever NDI video is LIVE; its value is the number, or
// "n/a" when there is no trustworthy measurement (no fresh /ndi/time offset) —
// never a misleading residual-only figure. Non-video layouts leave frames
// not-live so the readout is absent.
//
// The real per-frame value needs a live NDI/WebRTC stream (the self-hosted
// `@synthetic-ndi` GPU lane). This deterministic test runs on the standard
// GitHub-hosted `e2e` lane: it drives the frames-live flag and the latency
// value via the stage test hooks (`__presenterStageSetNdiFramesLive` /
// `__presenterStageSetVideoLatency`) — the same signals the rVFC observer
// writes — and asserts the readout appears when video is live, shows the
// number when measurable and "n/a" when not, and disappears when video stops.
// The derivation math (residual + network → ms, n/a-without-network,
// Tailscale-≥-LAN) is unit-tested in `ndi_frame_stats.rs`.
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

/** Drive the stage-side video-latency value (the same signal the rVFC observer
 * writes). `null` clears it → the readout shows "n/a" while video is live. */
async function setVideoLatency(page: Page, ms: number | null): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetVideoLatency?: (v: number | null) => void;
      }
    ).__presenterStageSetVideoLatency?.(value);
  }, ms);
}

/** Drive the "NDI frames are presenting" flag (gates the readout's visibility,
 * the same signal the rVFC observer / proxy write per frame). */
async function setFramesLive(page: Page, live: boolean): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetNdiFramesLive?: (v: boolean) => void;
      }
    ).__presenterStageSetNdiFramesLive?.(value);
  }, live);
}

test("stage shows true server→display latency as a separate readout, with honest n/a", async ({
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

  // No NDI video flowing yet → the video readout is absent (not just empty).
  await expect(videoEl).toHaveCount(0);

  // Video goes live but no trustworthy measurement yet → the readout appears
  // showing "n/a" (honest), NOT a misleading number.
  await setFramesLive(stagePage, true);
  await expect(videoEl).toBeVisible();
  await expect(videoEl).toContainText(/server→displej\s*·\s*n\/a/);

  // A measured server→display latency arrives → the readout shows "<n> ms".
  await setVideoLatency(stagePage, 42);
  await expect(videoEl).toContainText(/server→displej\s*·\s*42\s*ms/);

  // The two readouts coexist as DISTINCT elements (video latency shown
  // SEPARATELY from connection latency, not combined).
  await expect(connectionEl).toContainText("CONNECTED");
  await expect(connectionEl).not.toContainText("displej");
  await expect(videoEl).not.toContainText("CONNECTED");

  // The value updates live (a later, larger figure).
  await setVideoLatency(stagePage, 137);
  await expect(videoEl).toContainText(/server→displej\s*·\s*137\s*ms/);

  // Measurement lost while video still live (offset aged out) → honest n/a,
  // never a stale-but-confident number.
  await setVideoLatency(stagePage, null);
  await expect(videoEl).toContainText(/server→displej\s*·\s*n\/a/);

  // Video stops (source deactivated) → the readout disappears; the connection
  // readout remains.
  await setFramesLive(stagePage, false);
  await expect(videoEl).toHaveCount(0);
  await expect(connectionEl).toBeVisible();

  // browser-console-zero-errors: no errors/warnings the whole time.
  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});
