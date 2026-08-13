import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

declare global {
  interface Window {
    // Test-only hook installed by `installDynamicOrientation` (#694): mutate the
    // faked `screen.orientation` at runtime and optionally fire a `change`
    // event, to exercise the flip watcher's response to orientation SEQUENCES
    // (a transient sensor flap on a rotation-locked phone vs a genuine settled
    // turn) — impossible to simulate with the static `mockScreenOrientationType`
    // fake below.
    __setOrientation?: (
      type: string | null,
      angle: number | null,
      fireChange: boolean,
    ) => void;
  }
}

/**
 * Override `window.screen.orientation` to report a fixed `type` (and a
 * matching `angle`) BEFORE any page script runs — including this crate's own
 * `install_orientation_flip_watcher()` JS (`tablet_orientation.rs`), which
 * reads it at mount. A real device's physical rotation cannot be simulated
 * in headless Chromium, so this is the only way to exercise the
 * landscape-primary/landscape-secondary distinction end-to-end (#638).
 */
async function mockScreenOrientationType(
  page: Page,
  orientationType: "landscape-primary" | "landscape-secondary",
): Promise<void> {
  await page.addInitScript((type) => {
    const fake = {
      type,
      angle: type === "landscape-secondary" ? 180 : 0,
      addEventListener: () => {},
      removeEventListener: () => {},
    };
    Object.defineProperty(window.screen, "orientation", {
      configurable: true,
      get: () => fake,
    });
  }, orientationType);
}

/**
 * Install a MUTABLE fake `window.screen.orientation` (a real EventTarget-like
 * object) and expose `window.__setOrientation(type, angle, fireChange)` so a
 * test can drive orientation SEQUENCES at runtime — a genuine settled turn, or
 * the transient sensor flap a rotation-locked phone emits when lifted / laid
 * flat. The initial `type` may be `null` to emulate an engine that exposes only
 * `.angle` (no `.type`). Installed BEFORE any page script runs, so the crate's
 * own `install_orientation_flip_watcher()` binds to this fake at mount (#694).
 */
async function installDynamicOrientation(
  page: Page,
  initialType: string | null,
  initialAngle: number | null,
): Promise<void> {
  await page.addInitScript((init) => {
    let currentType = init.type;
    let currentAngle = init.angle;
    const changeListeners: Array<(ev: Event) => void> = [];
    const fake = {
      get type() {
        return currentType;
      },
      get angle() {
        return currentAngle;
      },
      addEventListener(name: string, cb: (ev: Event) => void) {
        if (name === "change" && typeof cb === "function") {
          changeListeners.push(cb);
        }
      },
      removeEventListener(name: string, cb: (ev: Event) => void) {
        if (name !== "change") {
          return;
        }
        const idx = changeListeners.indexOf(cb);
        if (idx >= 0) {
          changeListeners.splice(idx, 1);
        }
      },
    };
    window.__setOrientation = (type, angle, fireChange) => {
      currentType = type;
      currentAngle = angle;
      if (fireChange) {
        const ev = new Event("change");
        for (const cb of changeListeners.slice()) {
          cb.call(fake, ev);
        }
      }
    };
    Object.defineProperty(window.screen, "orientation", {
      configurable: true,
      get: () => fake,
    });
  }, { type: initialType, angle: initialAngle });
}

// ─────────────────────────────────────────────────────────────────────────
// #569 — the tablet UI (`/ui/tablet`) must stay pinned to a single
// orientation and never flip with phone position, even in the plain
// browser-tab flow (not installed as a PWA).
//
// Three layers:
//   1. The PWA manifest declares `"orientation": "landscape"` (was "any") —
//      honored automatically by an installed/standalone PWA.
//   2. A first-tap gesture (tablet_orientation.rs) attempts fullscreen +
//      `screen.orientation.lock("landscape")` for the plain-tab flow. Real
//      fullscreen/orientation-lock behavior is environment-dependent
//      (headless Chromium, no real screen) and not meaningfully assertable
//      here — the CSS fallback below is what's E2E-verifiable and is the
//      layer that actually guarantees the "never flips" requirement when
//      the lock is unsupported/denied/never attempted.
//   3. A pure CSS `@media (orientation: portrait)` fallback (tablet.css)
//      rotates the whole UI 90° so it keeps rendering in its landscape
//      design orientation regardless of the physical/window orientation.
//      This re-evaluates automatically on viewport resize — exactly what
//      Playwright's `setViewportSize` triggers — so it's fully testable via
//      viewport emulation without a real phone.
// ─────────────────────────────────────────────────────────────────────────

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("tablet manifest declares a fixed landscape-primary orientation (#569, #638)", async ({
  request,
}) => {
  const response = await request.get(
    new URL("/ui/tablet/manifest.json", baseURL).toString(),
  );
  expect(response.ok()).toBeTruthy();
  const manifest = await response.json();
  // #638: generic "landscape" permits EITHER landscape-primary or
  // landscape-secondary — an installed/standalone PWA could still honor a
  // physical 180° turn. "landscape-primary" pins the exact single
  // orientation, closing that gap for the installed-PWA flow.
  expect(manifest.orientation).toBe("landscape-primary");
});

// A real phone/tablet has a COARSE (touch) primary pointer — Playwright only
// makes `(pointer: coarse)` match when the browser context declares touch
// support, so this test explicitly emulates one (review finding, PR #579:
// the fallback is scoped to `(pointer: coarse)` so a desktop user with a
// narrow/portrait-shaped browser window is never force-rotated — see the
// sibling test below for that case).
test.describe("touch device (pointer: coarse)", () => {
  test.use({ hasTouch: true });

  test("tablet UI counter-rotates to stay landscape when the window is portrait, and does not when it's landscape (#569)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Start landscape (a typical tablet shape) — the fallback rotation must
    // NOT be active here; this is the everyday, already-correct case.
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).toBeAttached();

    const readTransform = () =>
      body.evaluate((el) => window.getComputedStyle(el).transform);
    const readPosition = () =>
      body.evaluate((el) => window.getComputedStyle(el).position);

    await expect.poll(readTransform).toBe("none");

    // Rotate to a portrait phone shape (a physically-rotated phone with no
    // orientation lock active — the exact bug condition: the UI must NOT
    // follow this rotation). The media query re-evaluates automatically.
    await page.setViewportSize({ width: 400, height: 800 });
    await expect.poll(readTransform).not.toBe("none");
    expect(await readPosition()).toBe("fixed");

    // Rotate back to landscape (phone turned back) — the fallback must
    // disengage cleanly, proving this is a live, bounded, reversible rule
    // and not a one-shot state that gets stuck.
    await page.setViewportSize({ width: 900, height: 500 });
    await expect.poll(readTransform).toBe("none");

    expect(consoleMessages).toEqual([]);
  });
});

// Desktop browsers report a FINE (mouse/trackpad) primary pointer regardless
// of window aspect ratio. A narrow/portrait-shaped desktop browser window
// (e.g. a portrait external monitor, a docked side-panel browser) must NOT
// get the tablet UI force-rotated — that would be a real, if unlikely,
// regression of the "no regression on desktop" requirement.
test("desktop (fine pointer) with a portrait-shaped window is NOT rotated (#569)", async ({
  page,
}) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  const body = page.locator("body.tablet");
  await expect(body).toBeAttached();
  const transform = await body.evaluate(
    (el) => window.getComputedStyle(el).transform,
  );
  expect(transform).toBe("none");
});

// ─────────────────────────────────────────────────────────────────────────
// #638 — the CSS `orientation` media feature above is 2-state
// (portrait/landscape) and cannot see a tablet physically turned 180° while
// STAYING landscape-shaped (landscape-primary ↔ landscape-secondary: width
// stays greater than height in both, so the query's premise never changes).
// `install_orientation_flip_watcher()` (tablet_orientation.rs) closes that
// gap using `screen.orientation.type` — the one API that actually
// distinguishes primary from secondary — mirrored onto
// `body[data-tablet-flip]`, counter-rotated 180° by a new tablet.css rule.
// ─────────────────────────────────────────────────────────────────────────
test.describe("touch device (pointer: coarse) — 180° flip (#638)", () => {
  test.use({ hasTouch: true });

  test("tablet UI counter-rotates 180° when the device reports landscape-secondary", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await mockScreenOrientationType(page, "landscape-secondary");
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).toHaveAttribute("data-tablet-flip", "true");

    const transform = await body.evaluate(
      (el) => window.getComputedStyle(el).transform,
    );
    expect(transform).not.toBe("none");

    expect(consoleMessages).toEqual([]);
  });

  test("tablet UI does NOT counter-rotate when the device reports landscape-primary", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await mockScreenOrientationType(page, "landscape-primary");
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).not.toHaveAttribute("data-tablet-flip", "true");

    const transform = await body.evaluate(
      (el) => window.getComputedStyle(el).transform,
    );
    expect(transform).toBe("none");

    expect(consoleMessages).toEqual([]);
  });
});

// Desktop browsers report a FINE (mouse/trackpad) primary pointer regardless
// of `screen.orientation.type` — a narrow/portrait-shaped OR a
// landscape-secondary-reporting desktop window must NEVER get force-rotated
// (same desktop-safety guarantee as the existing portrait fallback — #569
// review finding, PR #579).
test("desktop (fine pointer) reporting landscape-secondary is NOT rotated (#638)", async ({
  page,
}) => {
  await mockScreenOrientationType(page, "landscape-secondary");
  await page.setViewportSize({ width: 900, height: 500 });
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  const body = page.locator("body.tablet");
  await expect(body).not.toHaveAttribute("data-tablet-flip", "true");
  const transform = await body.evaluate(
    (el) => window.getComputedStyle(el).transform,
  );
  expect(transform).toBe("none");
});

// ─────────────────────────────────────────────────────────────────────────
// #694 — a rotation-LOCKED phone still flipped 180° when lifted / laid flat,
// because #638's watcher counter-rotated from a single INSTANTANEOUS read of
// `screen.orientation`. Per the W3C Screen Orientation spec, `screen.orientation`
// tracks the device's PHYSICAL orientation and fires `change` on physical tilt,
// while an OS rotation lock keeps only the DISPLAYED viewport fixed — so lifting
// / laying the phone flat makes the sensor transiently report landscape-secondary
// (or, on engines exposing only `.angle`, angle 180 = portrait-secondary on a
// natural-portrait phone, which the old `.angle === 180` fallback mis-mapped to a
// 180° flip) with the viewport never actually rotating. The distinguisher between
// this false trigger and a genuine turn (#638, AC3) is STABILITY: a real turn
// settles at secondary and stays; a lift/put-down flap reverts. The fix trusts
// only a debounced, settled `.type === "landscape-secondary"` and drops the
// resize trigger + the `.angle` fallback.
// ─────────────────────────────────────────────────────────────────────────
test.describe("touch device (pointer: coarse) — rotation-locked phone must not false-flip (#694)", () => {
  test.use({ hasTouch: true });

  // RED (#694): a transient landscape-secondary flap that reverts before the
  // watcher's stability window — the exact lift/put-down signal on a
  // rotation-locked phone (viewport never resizes; OS rotation is locked).
  test("a transient screen.orientation flap to landscape-secondary that reverts does NOT flip", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await installDynamicOrientation(page, "landscape-primary", 90);
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).toBeAttached();

    // Lift/put-down: a `change` fires reporting landscape-secondary, then the
    // device settles right back to landscape-primary before the watcher's
    // stability window elapses. The viewport dimensions never change.
    await page.evaluate(() =>
      window.__setOrientation!("landscape-secondary", 270, true),
    );
    await page.evaluate(() =>
      window.__setOrientation!("landscape-primary", 90, false),
    );

    // Wait past the stability window and assert the flip was never applied. A
    // fixed wait is correct here: we are proving a DEBOUNCED action does NOT
    // occur within its own settle boundary.
    await page.waitForTimeout(700);
    await expect(body).not.toHaveAttribute("data-tablet-flip", "true");
    expect(
      await body.evaluate((el) => window.getComputedStyle(el).transform),
    ).toBe("none");

    expect(consoleMessages).toEqual([]);
  });

  // RED (#694): an engine exposing only `screen.orientation.angle` (no `.type`)
  // reporting angle 180 at a landscape viewport. On a natural-portrait phone
  // angle 180 is portrait-secondary, NOT landscape-secondary (= 270°), so the
  // old `.angle === 180` fallback mis-mapped it to a 180° counter-rotate.
  test("an engine exposing only .angle (angle 180) at a landscape viewport does NOT flip", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await installDynamicOrientation(page, null, 180);
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).toBeAttached();

    await page.waitForTimeout(700); // past the stability window
    await expect(body).not.toHaveAttribute("data-tablet-flip", "true");
    expect(
      await body.evaluate((el) => window.getComputedStyle(el).transform),
    ).toBe("none");

    expect(consoleMessages).toEqual([]);
  });

  // Regression guard (#638, AC3): a genuine, SETTLED physical 180° turn of an
  // UNLOCKED tablet (screen.orientation fires `change` and stays at
  // landscape-secondary) must still counter-rotate — the fix must not regress
  // #638 in the opposite direction.
  test("a genuine settled 180° turn to landscape-secondary still counter-rotates", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await installDynamicOrientation(page, "landscape-primary", 90);
    await page.setViewportSize({ width: 900, height: 500 });
    await page.goto(new URL("/ui/tablet", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    const body = page.locator("body.tablet");
    await expect(body).not.toHaveAttribute("data-tablet-flip", "true");

    // Genuine turn: settles at landscape-secondary and STAYS there.
    await page.evaluate(() =>
      window.__setOrientation!("landscape-secondary", 270, true),
    );

    // The watcher applies the counter-rotate once the reading is stable
    // (toHaveAttribute polls, covering the debounce window).
    await expect(body).toHaveAttribute("data-tablet-flip", "true");
    expect(
      await body.evaluate((el) => window.getComputedStyle(el).transform),
    ).not.toBe("none");

    expect(consoleMessages).toEqual([]);
  });
});
