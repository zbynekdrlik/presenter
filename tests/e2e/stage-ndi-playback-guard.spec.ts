import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #568 — the native browser play-arrow overlay must NEVER appear on the
// stage NDI video, on ANY of the three stage layouts that mount <NdiVideo>
// (ndi-fullscreen, timer, api). Two root causes fixed:
//
// 1. The prior CSS suppression (`::-webkit-media-controls*`) targeted only
//    `.stage-ndi__video` (the ndi-fullscreen layout's class) — the timer
//    and api layouts' own NdiVideo classes had NO suppression at all.
// 2. Nothing re-initiated playback when the <video> element ended up in a
//    paused state (play() rejection, a stalled WHEP stream, a
//    background/foreground app-suspend cycle) — the browser then draws its
//    native "start playback" overlay over a paused video.
//
// This runs on the standard GitHub-hosted `e2e` lane (no NDI SDK/GPU
// needed): the CSS check only needs the stylesheet + the mounted <video>
// element (no real stream), and the pause-recovery check bypasses WHEP
// entirely by assigning a synthetic `canvas.captureStream()` MediaStream
// directly onto the mounted element — so it forces a REAL playing→paused→
// (guard fires)→playing cycle without needing a live NDI source.
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

/** Console noise expected on a host with no live NDI source/SDK: the WHEP
 * POST is answered 503 (no SDK) / 204 (configured-but-not-producing, #431)
 * and the client backs off quietly with a WARN — same allow-list used by the
 * existing timer/api NDI specs. */
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
 * activate succeeds without ever starting a real pipeline), select the given
 * stage layout, and open `/stage` ready + with `<video data-role="ndi-video">`
 * mounted. Returns the created source id (caller cleans it up). */
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

/** Find every `-webkit-media-controls*` CSS rule whose selector matches
 * `videoSelector`'s element and returns `display: none`. Checking the actual
 * stylesheet rule (rather than `getComputedStyle(el, '::pseudo')`, which some
 * headless Chromium builds resolve inconsistently for internal UA
 * pseudo-elements) deterministically proves the suppression rule REACHES this
 * exact element — which is the thing #568 actually broke (the rule existed,
 * it just didn't target the timer/api layouts' elements). */
async function mediaControlsSuppressionApplies(
  page: Page,
  videoSelector: string,
): Promise<boolean> {
  return page.evaluate((sel) => {
    const el = document.querySelector(sel);
    if (!el) return false;
    let matched = false;
    for (const sheet of Array.from(document.styleSheets)) {
      let rules: CSSRuleList;
      try {
        rules = sheet.cssRules;
      } catch {
        continue; // cross-origin stylesheet — not ours, skip
      }
      for (const rule of Array.from(rules)) {
        if (!(rule instanceof CSSStyleRule)) continue;
        if (!rule.selectorText.includes("media-controls")) continue;
        const bases = rule.selectorText
          .split(",")
          .map((s) => s.trim().replace(/::-webkit-media-controls[a-z-]*/gi, ""))
          .filter((s) => s.length > 0);
        const targetsThisElement = bases.some((base) => {
          try {
            return el.matches(base);
          } catch {
            return false;
          }
        });
        if (
          targetsThisElement &&
          rule.style.getPropertyValue("display").trim() === "none"
        ) {
          matched = true;
        }
      }
    }
    return matched;
  }, videoSelector);
}

for (const layout of [
  { code: "ndi-fullscreen", label: "PlaybackGuardFullscreen" },
  { code: "timer", label: "PlaybackGuardTimer" },
  { code: "api", label: "PlaybackGuardApi" },
]) {
  test(`native play-arrow overlay is suppressed on the ${layout.code} stage layout (#568)`, async ({
    page,
  }) => {
    const consoleMessages = collectConsoleNoise(page);
    const sourceId = await openStageWithNdiVideo(page, layout.code, layout.label);
    try {
      const suppressed = await mediaControlsSuppressionApplies(
        page,
        '[data-role="ndi-video"]',
      );
      expect(
        suppressed,
        `${layout.code}: no display:none -webkit-media-controls* rule targets [data-role="ndi-video"]`,
      ).toBe(true);
      expect(consoleMessages).toEqual([]);
    } finally {
      await deactivateAndDelete(page, sourceId);
    }
  });
}

test("NdiVideo replays a stalled stream after an external pause() within the bounded retry window (#568)", async ({
  page,
}) => {
  const consoleMessages = collectConsoleNoise(page);
  const sourceId = await openStageWithNdiVideo(
    page,
    "ndi-fullscreen",
    "PlaybackGuardReplay",
  );

  try {
    // Bypass WHEP entirely: assign a synthetic canvas-captured MediaStream
    // directly onto the mounted <video> element. This exercises the SAME
    // pause/ended/suspend replay guard the real WHEP path uses (the guard is
    // installed unconditionally per <NdiVideo> mount, independent of how the
    // element got its srcObject), without needing a live NDI source or GPU.
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      const canvas = document.createElement("canvas");
      canvas.width = 16;
      canvas.height = 16;
      const ctx = canvas.getContext("2d")!;
      let toggle = false;
      ctx.fillStyle = "red";
      ctx.fillRect(0, 0, 16, 16);
      // Repaint periodically so the captured track has real frame changes —
      // avoids any "static canvas" edge case in some Chromium versions.
      const interval = setInterval(() => {
        toggle = !toggle;
        ctx.fillStyle = toggle ? "blue" : "red";
        ctx.fillRect(0, 0, 16, 16);
      }, 100);
      (window as unknown as { __e2eGuardInterval?: number }).__e2eGuardInterval =
        interval as unknown as number;
      const stream = canvas.captureStream(10);
      video.srcObject = stream;
      video.muted = true;
      return video.play();
    }, '[data-role="ndi-video"]');

    const isPaused = () =>
      page.evaluate(
        (sel) => (document.querySelector(sel) as HTMLVideoElement).paused,
        '[data-role="ndi-video"]',
      );

    // Sanity: the synthetic stream is genuinely playing before we pause it.
    await expect.poll(isPaused, { timeout: 10_000 }).toBe(false);

    // Record the `pause` EVENT directly (a discrete fact) rather than polling
    // the `paused` boolean for `true` — the fix under test reacts to `pause`
    // so fast that a poll can miss the brief paused window entirely and only
    // ever observe `false`, which would make this sanity check itself flaky.
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      (window as unknown as { __e2ePauseSeen?: boolean }).__e2ePauseSeen = false;
      video.addEventListener(
        "pause",
        () => {
          (window as unknown as { __e2ePauseSeen?: boolean }).__e2ePauseSeen = true;
        },
        { once: true },
      );
    }, '[data-role="ndi-video"]');

    // Force the exact bug condition: the element ends up paused (a rejected
    // play(), a stalled stream, or — as simulated here — any external pause).
    await page.evaluate(
      (sel) => (document.querySelector(sel) as HTMLVideoElement).pause(),
      '[data-role="ndi-video"]',
    );
    await expect
      .poll(() => page.evaluate(() => (window as unknown as { __e2ePauseSeen?: boolean }).__e2ePauseSeen), {
        timeout: 5_000,
      })
      .toBe(true);

    // The bounded replay guard (ndi_playback_guard.rs) must detect the
    // `pause` event and re-call `.play()` — recovering WITHOUT any reload or
    // reconnect. This is the #568 core fix: before it existed, the element
    // stayed paused forever and the browser drew its native play-arrow.
    await expect.poll(isPaused, { timeout: 5_000 }).toBe(false);

    await page.evaluate(() => {
      const w = window as unknown as { __e2eGuardInterval?: number };
      if (w.__e2eGuardInterval) clearInterval(w.__e2eGuardInterval);
    });

    expect(consoleMessages).toEqual([]);
  } finally {
    await deactivateAndDelete(page, sourceId);
  }
});
