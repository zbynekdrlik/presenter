import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

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
