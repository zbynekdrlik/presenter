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

/** Drive the recent-window stage-health verdict (#532) — the SAME reading
 * the beacon-driven classifier writes. `null` clears it. */
async function setStageHealth(
  page: Page,
  reading: { state: "good" | "degraded" | "bad"; fps: number } | null,
): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetHealth?: (
          state: string | null,
          fps: number | null,
        ) => void;
      }
    ).__presenterStageSetHealth?.(
      value ? value.state : null,
      value ? value.fps : null,
    );
  }, reading);
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
// #532 — the stage shows a RECENT-WINDOW health verdict (🟢/🟡/🔴) beside the
// latency figure, so "is this TV usable for the band right now" is visible at
// a glance. Replaces #523's CUMULATIVE ⬇N/❄N suffix (which never recovered
// from one old network blip — meaningless to an operator glancing at the
// stage). Sourced from the render-side per-interval accumulators (presented
// fps + presentation-gap stats), classified client-side; this test drives it
// via the deterministic test hook (`__presenterStageSetHealth`), the same
// signal the beacon-driven classifier writes. The fps/gap → verdict
// threshold math is unit-tested in `ndi_frame_stats.rs`; the text formatting
// is unit-tested in `status_bar.rs`.
// ─────────────────────────────────────────────────────────────────────────

test("stage shows a recent-window health verdict beside the video latency", async ({
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

  // No beacon has classified a window yet → the readout shows the latency
  // ALONE, no fabricated verdict.
  await expect(videoEl).not.toContainText("plynul");
  await expect(videoEl).not.toContainText("sek");
  await expect(videoEl).not.toContainText("výpadky");

  // A beacon classifies a smooth window → 🟢 plynulé + the recent fps.
  await setStageHealth(stagePage, { state: "good", fps: 28 });
  await expect(videoEl).toContainText("🟢");
  await expect(videoEl).toContainText("plynulé");
  await expect(videoEl).toContainText("28 fps");

  // A beacon classifies minor stutter → 🟡 mierne seká.
  await setStageHealth(stagePage, { state: "degraded", fps: 20 });
  await expect(videoEl).toContainText("🟡");
  await expect(videoEl).toContainText("mierne seká");
  await expect(videoEl).toContainText("20 fps");

  // A beacon classifies real freezing → 🔴 výpadky.
  await setStageHealth(stagePage, { state: "bad", fps: 6 });
  await expect(videoEl).toContainText("🔴");
  await expect(videoEl).toContainText("výpadky");
  await expect(videoEl).toContainText("6 fps");

  // Reconnect (or no classified window yet) clears it → readout falls back
  // to the latency alone, never a stale verdict.
  await setStageHealth(stagePage, null);
  await expect(videoEl).not.toContainText("výpadky");
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
