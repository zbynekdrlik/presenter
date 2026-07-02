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

/** Drive the "NDI source active" flag (gates the readout's visibility — the
 * stable per-layout signal, set from the live snapshot in production). */
async function setNdiActive(page: Page, active: boolean): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetNdiActive?: (v: boolean) => void;
      }
    ).__presenterStageSetNdiActive?.(value);
  }, active);
}

/** Drive the per-display dropped-frame + freeze counters (#523) — the SAME
 * pair the getStats beacon writes. `null` clears them. */
async function setDroppedFrames(
  page: Page,
  counts: { dropped: number; freeze: number } | null,
): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetDroppedFrames?: (
          dropped: number | null,
          freeze: number | null,
        ) => void;
      }
    ).__presenterStageSetDroppedFrames?.(
      value ? value.dropped : null,
      value ? value.freeze : null,
    );
  }, counts);
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

  // No NDI source active yet → the video readout is absent (not just empty).
  await expect(videoEl).toHaveCount(0);

  // NDI source goes active but no trustworthy measurement yet → the readout
  // appears showing "n/a" (honest), NOT a misleading number.
  await setNdiActive(stagePage, true);
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

  // NDI source deactivated → the readout disappears; the connection readout
  // remains.
  await setNdiActive(stagePage, false);
  await expect(videoEl).toHaveCount(0);
  await expect(connectionEl).toBeVisible();

  // browser-console-zero-errors: no errors/warnings the whole time.
  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

// ─────────────────────────────────────────────────────────────────────────
// #523 — the stage shows per-display dropped-frame (+freeze) counts beside
// the latency figure, so "how is this TV doing" is visible at a glance (a
// low latency reading can otherwise hide a TV that is dropping frames to
// achieve it). Sourced from the SAME getStats inbound-rtp sample the health
// beacon already reads; this test drives it via the deterministic test hook
// (`__presenterStageSetDroppedFrames`), the same signal the beacon path
// writes. The append-format math (⬇N, +❄N only when nonzero) is unit-tested
// in `status_bar.rs`.
// ─────────────────────────────────────────────────────────────────────────

test("stage shows dropped-frame + freeze count beside the video latency", async ({
  context,
}) => {
  const consoleMessages: string[] = [];
  const stagePage = await openVideoStage(context);
  stagePage.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const videoEl = stagePage.locator(".stage__video-latency");

  await setNdiActive(stagePage, true);
  await setVideoLatency(stagePage, 84);
  await expect(videoEl).toContainText(/server→displej\s*·\s*84\s*ms/);

  // No beacon has landed yet → the readout shows the latency ALONE, no
  // fabricated drop count.
  await expect(videoEl).not.toContainText("⬇");

  // A beacon lands with zero drops/freezes → shown as "⬇0" (honest zero, not
  // hidden — the whole point is a per-TV health signal at a glance).
  await setDroppedFrames(stagePage, { dropped: 0, freeze: 0 });
  await expect(videoEl).toContainText(/server→displej\s*·\s*84\s*ms\s*·\s*⬇0/);
  await expect(videoEl).not.toContainText("❄");

  // Drops accumulate → the count updates live.
  await setDroppedFrames(stagePage, { dropped: 128, freeze: 0 });
  await expect(videoEl).toContainText(/⬇128/);
  await expect(videoEl).not.toContainText("❄");

  // A freeze count is present too → shown alongside the drop count.
  await setDroppedFrames(stagePage, { dropped: 128, freeze: 2 });
  await expect(videoEl).toContainText(/⬇128\s*❄2/);

  // Reconnect (or no getStats data) clears it → readout falls back to the
  // latency alone, never a stale count.
  await setDroppedFrames(stagePage, null);
  await expect(videoEl).not.toContainText("⬇");
  await expect(videoEl).toContainText(/server→displej\s*·\s*84\s*ms/);

  // browser-console-zero-errors: no errors/warnings the whole time.
  expect(consoleMessages).toEqual([]);

  await stagePage.close();
});

// ─────────────────────────────────────────────────────────────────────────
// #524 — the diagnostic readouts (`.stage__connection`, `.stage__video-latency`)
// must render SMALL + FAINT (close-up info for the operator, not primary
// content) rather than autofit-scaled to fill their box (which is why they
// used to look too prominent). Verified by reading the COMPUTED style —
// asserting a fixed small font-size + low opacity, not just visual guessing.
// ─────────────────────────────────────────────────────────────────────────

test("diagnostic readouts render small and faint (de-emphasized, not autofit)", async ({
  context,
}) => {
  const stagePage = await openVideoStage(context);
  await setNdiActive(stagePage, true);
  await setVideoLatency(stagePage, 84);

  const connectionEl = stagePage.locator(".stage__connection");
  const videoEl = stagePage.locator(".stage__video-latency");
  await expect(videoEl).toBeVisible();

  const readComputed = async (locator: typeof connectionEl) =>
    locator.evaluate((el) => {
      const style = window.getComputedStyle(el);
      return { fontSize: parseFloat(style.fontSize), opacity: parseFloat(style.opacity) };
    });

  const connectionStyle = await readComputed(connectionEl);
  const videoStyle = await readComputed(videoEl);

  // Faint: low opacity (~0.4-0.5), never full-strength like primary content.
  expect(connectionStyle.opacity).toBeGreaterThan(0);
  expect(connectionStyle.opacity).toBeLessThanOrEqual(0.5);
  expect(videoStyle.opacity).toBeGreaterThan(0);
  expect(videoStyle.opacity).toBeLessThanOrEqual(0.5);

  // Small: a fixed vw-scaled size, not autofit-to-fill-the-box (which would
  // scale toward the STATUS_MAX_FONT ceiling). Comfortably below any autofit
  // result, but still nonzero (readable up close, per the issue).
  expect(connectionStyle.fontSize).toBeGreaterThan(0);
  expect(connectionStyle.fontSize).toBeLessThan(40);
  expect(videoStyle.fontSize).toBeGreaterThan(0);
  expect(videoStyle.fontSize).toBeLessThan(40);

  await stagePage.close();
});
