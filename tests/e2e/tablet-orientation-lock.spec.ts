import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

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

test("tablet manifest declares a fixed landscape orientation (#569)", async ({
  request,
}) => {
  const response = await request.get(
    new URL("/ui/tablet/manifest.json", baseURL).toString(),
  );
  expect(response.ok()).toBeTruthy();
  const manifest = await response.json();
  expect(manifest.orientation).toBe("landscape");
});

test("tablet UI counter-rotates to stay landscape when the window is portrait, and does not when it's landscape (#569)", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Start landscape (a typical tablet/desktop shape) — the fallback rotation
  // must NOT be active here; this is the everyday, already-correct case.
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
  // disengage cleanly, proving this is a live, bounded, reversible rule and
  // not a one-shot state that gets stuck.
  await page.setViewportSize({ width: 900, height: 500 });
  await expect.poll(readTransform).toBe("none");

  expect(consoleMessages).toEqual([]);
});
