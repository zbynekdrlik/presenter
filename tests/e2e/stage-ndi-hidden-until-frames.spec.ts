import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #732 — the grey play-arrow (4th recurrence of #448/#478/#568/#637).
//
// ROOT CAUSE, proven live on the real stage TV SD1 (Tesla/Skyworth LEAP-S1,
// Chrome/150 WebView): the arrow is `-internal-media-controls-overlay-play-
// button-internal` — a Chrome ≥150 UA-INTERNAL pseudo-element that author CSS
// CANNOT select — painted INSIDE an empty/frameless `<video data-role=
// "ndi-video">` (no srcObject, readyState 0, paused). The #478/#568
// `::-webkit-media-controls*` rules DO compute display:none on the field
// engine but only reach the `-webkit-`-named wrapper, never the `-internal-…`
// glyph → every "verified in Chromium" CSS fix left the field arrow unchanged.
// Proven on SD1 that `opacity:0` on the `<video>` ELEMENT removes it.
//
// THE FIX: NdiVideo hides the `<video>` element (class `stage-ndi-video--
// dormant` → opacity:0) whenever frames are NOT presenting, revealing it only
// when real frames flow — gated on the SAME `#500 ndi_frames_live` signal the
// rVFC frame observer writes (flipped false after FRAMES_LIVE_STALENESS_MS of
// no frames / on cleanup / on (de)activate). The mechanism lives in NdiVideo,
// so ALL THREE layouts (ndi-fullscreen, timer, api) inherit it — the arrow
// surfaced on the coverless timer/api layouts and in every layout's cold-open
// WHEP window.
//
// This runs on the standard GitHub-hosted `e2e` lane (no NDI SDK/GPU): the
// source activates without a real pipeline (holds the neutral no-frames
// state), and the "frames are presenting" flag is driven via the stage test
// hook `__presenterStageSetNdiFramesLive` — the EXACT signal the rVFC observer
// writes per frame — so the dormant→visible→dormant transition is exercised
// deterministically without a live NDI source.
//
// Visibility is asserted via the class attribute + getComputedStyle().opacity,
// NEVER Playwright toBeVisible() — an opacity:0 element still reports visible
// (see .claude/skills/ui/SKILL.md).
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

// Console noise expected on a host with no live NDI source/SDK: WHEP POST is
// answered 503 (no SDK) / 204 (configured-but-not-producing, #431) and the
// client backs off quietly with a WARN — same allow-list as the existing
// playback-guard / frames-live-cover specs.
const ALLOWED_CONSOLE_NOISE = [
  /Failed to load resource.*\b(503|204)\b/i,
  /WHEP (POST|connect)[^\n]*\b(503|204)\b/i,
  /reconnect_loop.*connect_whep failed/i,
];

function collectConsoleNoise(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    const text = msg.text();
    if (!ALLOWED_CONSOLE_NOISE.some((re) => re.test(text))) {
      messages.push(`[${msg.type()}] ${text}`);
    }
  });
  page.on("pageerror", (err) => messages.push(`[pageerror] ${err.message}`));
  return messages;
}

/** Create + activate a not-producing NDI source (no SDK on this runner →
 * activate succeeds without ever starting a real pipeline, so the client holds
 * the neutral no-frames state), select the given stage layout, and open
 * `/stage` ready + with `<video data-role="ndi-video">` mounted. Returns the
 * created source id (caller cleans it up). */
async function openStageWithNdiVideo(
  page: Page,
  layoutCode: string,
  label: string,
): Promise<string> {
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    { failOnStatusCode: false },
  );
  await page.request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: layoutCode },
  });

  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label, ndiName: `BOGUS-${label}` } },
  );
  const source = await created.json();
  await page.request.post(
    new URL(
      `/integrations/video-sources/${source.id}/activate`,
      baseURL,
    ).toString(),
    { failOnStatusCode: false },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector(`body[data-layout-code="${layoutCode}"]`, {
    timeout: 10_000,
  });
  // The element is present in the DOM even while dormant (opacity:0 is still
  // selectable) — waitForSelector checks presence, not visibility.
  await page.waitForSelector('[data-role="ndi-video"]', { timeout: 15_000 });

  return source.id as string;
}

async function deactivateAndDelete(page: Page, sourceId: string): Promise<void> {
  await page.request.post(
    new URL("/integrations/video-sources/deactivate", baseURL).toString(),
    { failOnStatusCode: false },
  );
  await page.request.delete(
    new URL(`/integrations/video-sources/${sourceId}`, baseURL).toString(),
    { failOnStatusCode: false },
  );
}

/** Drive the "frames are presenting" flag — the SAME signal the rVFC frame
 * observer writes per frame (StageContext::ndi_frames_live). */
async function setNdiFramesLive(page: Page, live: boolean): Promise<void> {
  await page.evaluate((value) => {
    (
      window as unknown as {
        __presenterStageSetNdiFramesLive?: (v: boolean) => void;
      }
    ).__presenterStageSetNdiFramesLive?.(value);
  }, live);
}

/** Computed opacity of the mounted NDI `<video>` element — the property the
 * fix drives. Class-attribute assertions use toHaveClass separately; opacity
 * is read here because an opacity:0 element still passes toBeVisible(). */
async function ndiVideoOpacity(page: Page): Promise<string> {
  return page.evaluate(() => {
    const el = document.querySelector('[data-role="ndi-video"]');
    if (!el) return "missing";
    return getComputedStyle(el).opacity;
  });
}

for (const layout of [
  { code: "ndi-fullscreen", label: "HiddenUntilFramesFullscreen" },
  { code: "timer", label: "HiddenUntilFramesTimer" },
  { code: "api", label: "HiddenUntilFramesApi" },
]) {
  test(`NDI <video> is hidden until frames present on the ${layout.code} stage layout (#732)`, async ({
    page,
  }) => {
    const consoleMessages = collectConsoleNoise(page);
    const sourceId = await openStageWithNdiVideo(page, layout.code, layout.label);
    const video = page.locator('[data-role="ndi-video"]');

    try {
      // ── RED: active source, NO frames flowing → the frameless <video> would
      // paint Chrome-150's `-internal-…-play-button-internal` grey arrow. The
      // element MUST be dormant (opacity:0) so nothing paints. FAILS on
      // current code (video is fully visible, no dormant class).
      await expect(video).toHaveClass(/stage-ndi-video--dormant/, {
        timeout: 10_000,
      });
      await expect.poll(() => ndiVideoOpacity(page), { timeout: 10_000 }).toBe(
        "0",
      );

      // ── GREEN: frames start presenting (rVFC would fire) → the element is
      // revealed so the real video shows.
      await setNdiFramesLive(page, true);
      await expect(video).not.toHaveClass(/stage-ndi-video--dormant/, {
        timeout: 10_000,
      });
      await expect.poll(() => ndiVideoOpacity(page), { timeout: 10_000 }).toBe(
        "1",
      );
      // Element stays mounted the whole time (WHEP negotiation untouched).
      await expect(video).toHaveCount(1);

      // ── Teardown: frames stop (source silent / stalled / watchdog no-frame
      // reconnect) → the element goes dormant again so the arrow never
      // reappears on the frozen/empty video.
      await setNdiFramesLive(page, false);
      await expect(video).toHaveClass(/stage-ndi-video--dormant/, {
        timeout: 10_000,
      });
      await expect.poll(() => ndiVideoOpacity(page), { timeout: 10_000 }).toBe(
        "0",
      );

      // browser-console-zero-errors: clean console the whole time.
      expect(consoleMessages).toEqual([]);
    } finally {
      await deactivateAndDelete(page, sourceId);
    }
  });
}
