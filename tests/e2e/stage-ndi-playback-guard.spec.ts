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

// #568 review follow-up (PR #579): the sibling test above proves the
// pause/ended/suspend replay guard recovers from an EXPLICIT pause() call,
// but Playwright's default `chromium` project disables Chrome's autoplay
// policy entirely (see playwright.config.ts), so it can't prove the replay
// still succeeds under REAL enforcement — the ticket's #1 named root cause
// ("video.play() rejects (autoplay policy)... and no retry brings it back").
// This re-runs the same playing→paused→recovered cycle under real Chrome
// with `--autoplay-policy=user-gesture-required` (the `chrome-video`
// project, `@video-codec` tag — same mechanism as the pre-existing
// "NdiVideo actually starts playing (autoplay policy regression)" test in
// ndi-webrtc.spec.ts), proving the guard's `set_muted(true)` + `.play()`
// re-assertion (ndi_playback_guard.rs) genuinely survives strict policy
// enforcement — a script-initiated replay is ONLY permitted at all because
// it re-asserts `muted` first.
test("NdiVideo replays a stalled stream after pause() under real Chrome autoplay-policy enforcement (#568) @video-codec", async ({
  page,
}) => {
  const consoleMessages = collectConsoleNoise(page);
  const sourceId = await openStageWithNdiVideo(
    page,
    "ndi-fullscreen",
    "PlaybackGuardReplayStrictPolicy",
  );

  try {
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      const canvas = document.createElement("canvas");
      canvas.width = 16;
      canvas.height = 16;
      const ctx = canvas.getContext("2d")!;
      let toggle = false;
      ctx.fillStyle = "red";
      ctx.fillRect(0, 0, 16, 16);
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

    // Under REAL Chrome with --autoplay-policy=user-gesture-required, a
    // programmatic play() on a NON-muted element would reject — this first
    // poll proves the initial muted play() succeeds even under strict
    // enforcement (mirrors attach_ontrack's production behavior).
    await expect.poll(isPaused, { timeout: 10_000 }).toBe(false);

    await page.evaluate(
      (sel) => (document.querySelector(sel) as HTMLVideoElement).pause(),
      '[data-role="ndi-video"]',
    );

    // The guard's replay must recover even here — proving the fix holds
    // under real policy enforcement, not just Playwright's relaxed default.
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

// ─────────────────────────────────────────────────────────────────────────
// #637 — `install()` (ndi_playback_guard.rs) `.forget()`-leaked its
// pause/ended/suspend/visibilitychange listeners under the false assumption
// that <NdiVideo> mounts exactly once per page load. It does not:
// ndi_fullscreen.rs's `<Show when=move || ndi_active.get()>` unmounts and
// remounts <NdiVideo> on every source (de)activation, so a long-running
// stage display accumulated one dead listener set (plus one detached
// <video> element handle) per cycle, forever.
//
// This proves the fix by instrumenting `EventTarget.prototype.add/removeEventListener`
// BEFORE the WASM bundle boots (page.addInitScript), tracking the NET
// (adds - removes) count for the guard's 4 event types. Only
// `ndi_playback_guard.rs` registers "pause"/"ended"/"suspend" anywhere in
// this crate; "visibilitychange" is ALSO registered once, page-scoped, by
// `wake_lock.rs` (correctly `.forget()`-leaked there — StagePage itself
// never remounts), which is why the visibilitychange baseline is 1 instead
// of 0. A leak shows these counts climbing by one every remount cycle
// instead of returning to their post-first-mount baseline.
// ─────────────────────────────────────────────────────────────────────────

type TrackedEventName = "pause" | "ended" | "suspend" | "visibilitychange";
type ListenerNetCounts = Record<TrackedEventName, number>;

test("ndi_playback_guard removes its listeners on unmount — no leak across remount cycles (#637)", async ({
  page,
}) => {
  const consoleMessages = collectConsoleNoise(page);

  // Installed before ANY page script runs, so it also catches the very
  // first NdiVideo mount's listener registrations.
  await page.addInitScript(() => {
    const proto = EventTarget.prototype;
    const originalAdd = proto.addEventListener;
    const originalRemove = proto.removeEventListener;
    const tracked = new Set(["pause", "ended", "suspend", "visibilitychange"]);
    const counts: Record<string, number> = {
      pause: 0,
      ended: 0,
      suspend: 0,
      visibilitychange: 0,
    };
    (window as unknown as { __e2eListenerNet: Record<string, number> }).__e2eListenerNet =
      counts;
    proto.addEventListener = function (
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | AddEventListenerOptions,
    ) {
      if (tracked.has(type)) counts[type] = (counts[type] ?? 0) + 1;
      return originalAdd.call(this, type, listener, options);
    };
    proto.removeEventListener = function (
      this: EventTarget,
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: boolean | EventListenerOptions,
    ) {
      if (tracked.has(type)) counts[type] = (counts[type] ?? 0) - 1;
      return originalRemove.call(this, type, listener, options);
    };
  });

  const sourceId = await openStageWithNdiVideo(page, "ndi-fullscreen", "PlaybackGuardLeak");
  const video = page.locator('[data-role="ndi-video"]');

  const netCounts = (): Promise<ListenerNetCounts> =>
    page.evaluate(
      () =>
        (window as unknown as { __e2eListenerNet: ListenerNetCounts }).__e2eListenerNet,
    );

  try {
    // After the FIRST mount: one guard listener set installed (+1 each for
    // pause/ended/suspend), plus wake_lock's own page-level visibilitychange
    // listener (+1, installed once, correctly never removed) + this guard
    // mount's own visibilitychange (+1) = 2.
    await expect.poll(async () => (await netCounts()).pause, { timeout: 10_000 }).toBe(1);
    let counts = await netCounts();
    expect(counts.ended, "ended listener not installed on first mount").toBe(1);
    expect(counts.suspend, "suspend listener not installed on first mount").toBe(1);
    expect(
      counts.visibilitychange,
      "visibilitychange net count after first mount should be wake_lock(1) + guard(1)",
    ).toBe(2);

    for (let cycle = 0; cycle < 3; cycle += 1) {
      // Unmount: deactivate the source → ndi_active flips false → <Show>
      // tears down <NdiVideo> → on_cleanup must remove the guard's listeners.
      await page.request.post(
        new URL("/integrations/video-sources/deactivate", baseURL).toString(),
        { failOnStatusCode: false },
      );
      await expect(video).toHaveCount(0, { timeout: 10_000 });

      await expect
        .poll(async () => (await netCounts()).pause, {
          timeout: 10_000,
          message: `cycle ${cycle}: pause listener not removed on unmount (leak)`,
        })
        .toBe(0);
      counts = await netCounts();
      expect(counts.ended, `cycle ${cycle}: ended listener not removed on unmount (leak)`).toBe(
        0,
      );
      expect(
        counts.suspend,
        `cycle ${cycle}: suspend listener not removed on unmount (leak)`,
      ).toBe(0);
      // Only THIS guard's own visibilitychange listener must be gone —
      // wake_lock's page-level one (installed once, at StagePage mount)
      // correctly remains for the page's lifetime.
      expect(
        counts.visibilitychange,
        `cycle ${cycle}: guard's own visibilitychange listener not removed on unmount (leak)`,
      ).toBe(1);

      // Remount: reactivate the same source.
      await page.request.post(
        new URL(`/integrations/video-sources/${sourceId}/activate`, baseURL).toString(),
        { failOnStatusCode: false },
      );
      await expect(video).toBeVisible({ timeout: 15_000 });

      await expect
        .poll(async () => (await netCounts()).pause, {
          timeout: 10_000,
          message: `cycle ${cycle}: pause listener count grew across remount (leak)`,
        })
        .toBe(1);
      counts = await netCounts();
      expect(
        counts.ended,
        `cycle ${cycle}: ended listener count grew across remount (leak)`,
      ).toBe(1);
      expect(
        counts.suspend,
        `cycle ${cycle}: suspend listener count grew across remount (leak)`,
      ).toBe(1);
      expect(
        counts.visibilitychange,
        `cycle ${cycle}: visibilitychange listener count grew across remount (leak)`,
      ).toBe(2);
    }

    expect(consoleMessages).toEqual([]);
  } finally {
    await deactivateAndDelete(page, sourceId);
  }
});

// ─────────────────────────────────────────────────────────────────────────
// #637 — the `@video-codec` test above ("replays a stalled stream after
// pause() under real Chrome autoplay-policy enforcement") sets
// `video.muted = true` before BOTH the initial and the replay play(), and
// `replay_if_within_budget` (ndi_playback_guard.rs) ALSO force-re-asserts
// `muted = true` before every replay play() call — so no test anywhere in
// this suite ever produces a play() call the autoplay policy would reject.
// The Err(e) branch of `play_and_log` on a genuinely REJECTED promise
// (ndi_playback_guard.rs, inside the `Ok(promise) => spawn_local(...
// JsFuture::from(promise) ...)` arm) is therefore never exercised.
//
// This test mocks `HTMLMediaElement.prototype.play` — scoped to ONLY the
// target video element, delegating to the real implementation for anything
// else — so it deterministically returns a rejected promise regardless of
// mute state, then triggers the guard's replay path via pause() and asserts
// the resulting "play() rejected" WARN is actually observed in the console.
// ─────────────────────────────────────────────────────────────────────────

test("ndi_playback_guard logs a warning when the replay play() promise genuinely rejects (#637)", async ({
  page,
}) => {
  const rejectionMessages: string[] = [];
  const unexpectedMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    const text = msg.text();
    if (/ndi_playback_guard:.*play\(\) rejected/i.test(text)) {
      rejectionMessages.push(text);
      return;
    }
    if (!ALLOWED_CONSOLE_NOISE.some((re) => re.test(text))) {
      unexpectedMessages.push(`[${msg.type()}] ${text}`);
    }
  });
  page.on("pageerror", (err) => unexpectedMessages.push(`[pageerror] ${err.message}`));

  const sourceId = await openStageWithNdiVideo(
    page,
    "ndi-fullscreen",
    "PlaybackGuardRejectedPromise",
  );

  try {
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      const canvas = document.createElement("canvas");
      canvas.width = 16;
      canvas.height = 16;
      const ctx = canvas.getContext("2d")!;
      let toggle = false;
      ctx.fillStyle = "red";
      ctx.fillRect(0, 0, 16, 16);
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

    // Sanity: genuinely playing before we mock the rejection.
    await expect.poll(isPaused, { timeout: 10_000 }).toBe(false);

    // Mock play() for THIS element only — every other element (none exist
    // on this bare /stage page, but kept for safety) still gets the real
    // implementation.
    await page.evaluate((sel) => {
      const video = document.querySelector(sel) as HTMLVideoElement;
      const proto = HTMLMediaElement.prototype as unknown as {
        play: (this: HTMLMediaElement) => Promise<void>;
      };
      const originalPlay = proto.play;
      proto.play = function (this: HTMLMediaElement) {
        if (this === video) {
          return Promise.reject(
            new DOMException("mocked play() rejection (#637 test)", "NotAllowedError"),
          );
        }
        return originalPlay.call(this);
      };
    }, '[data-role="ndi-video"]');

    await page.evaluate(
      (sel) => (document.querySelector(sel) as HTMLVideoElement).pause(),
      '[data-role="ndi-video"]',
    );

    // The guard's replay play() now genuinely rejects — this proves the
    // Err(e) branch in play_and_log actually runs and logs it, which the
    // muted replay in the sibling tests above can never exercise (a muted
    // play() never rejects under Chrome's autoplay policy).
    await expect
      .poll(() => rejectionMessages.length, {
        timeout: 5_000,
        message: "play_and_log never logged a play() rejected warning",
      })
      .toBeGreaterThan(0);

    await page.evaluate(() => {
      const w = window as unknown as { __e2eGuardInterval?: number };
      if (w.__e2eGuardInterval) clearInterval(w.__e2eGuardInterval);
    });

    expect(unexpectedMessages).toEqual([]);
  } finally {
    await deactivateAndDelete(page, sourceId);
  }
});
