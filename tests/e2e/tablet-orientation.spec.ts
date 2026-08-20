import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// ─────────────────────────────────────────────────────────────────────────
// #699 — the Bible Tablet (`/ui/tablet`) must FOLLOW device rotation and be
// usable in BOTH portrait and landscape. This REVERSES the #569/#638/#694
// landscape-LOCK: the same owner originally asked for a fixed landscape
// orientation, and now (verbatim: "bible tablet nefunguje otacanie, vzdy je
// webka iba na sirku, a chcem ju vediet mat aj na sirku") wants rotation to
// work. The old landscape-lock (a CSS 90° counter-rotation, a
// screen.orientation.lock gesture + 180° flip watcher, and a
// landscape-primary PWA manifest) is removed; the layout must render at the
// device's native orientation, unrotated, in both shapes. Desktop
// (fine-pointer) was never force-rotated (every lock rule was gated on
// `(pointer: coarse)`) and must stay untouched.
//
// The old spec `tablet-orientation-lock.spec.ts` asserted the now-reversed
// LOCK behavior and is deleted with this file (tdd-workflow: a test encoding
// behavior the owner has explicitly reversed is replaced, with justification).
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

test("tablet manifest allows any orientation, following the device (#699)", async ({
  request,
}) => {
  const response = await request.get(
    new URL("/ui/tablet/manifest.json", baseURL).toString(),
  );
  expect(response.ok()).toBeTruthy();
  const manifest = await response.json();
  // #699 reverses #569/#638's `"landscape-primary"` lock: the installed/
  // standalone PWA must now follow the device orientation, i.e. `"any"`.
  expect(manifest.orientation).toBe("any");
});

// A real phone/tablet has a COARSE (touch) primary pointer — Playwright only
// makes `(pointer: coarse)` match when the browser context declares touch
// support (see `.claude/skills/ui` #569 note). This is the side of the gate
// the old lock rules targeted, so it is where the reversal must be proven.
test.describe("touch device (pointer: coarse)", () => {
  test.use({ hasTouch: true });

  test("bible tablet follows device rotation — usable, unrotated, in BOTH portrait and landscape (#699)", async ({
    page,
  }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // Landscape (a typical tablet held wide) — must render natively, unrotated.
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

    // The header identifies the page and is a usability anchor for both shapes.
    const heading = page.locator(".tablet-header h1", { hasText: "Bible Tablet" });

    await expect.poll(readTransform).toBe("none");
    await expect(heading).toBeVisible();

    // Rotate to portrait (the device physically turned upright). This is the
    // exact bug condition: the UI must NOT force itself back to landscape — it
    // must follow the device. Currently a `@media (orientation: portrait) and
    // (pointer: coarse)` rule rotates `body.tablet` 90° and pins it
    // `position: fixed`; after the fix neither applies (RED before, GREEN after).
    await page.setViewportSize({ width: 400, height: 800 });
    await expect.poll(readTransform).toBe("none");
    expect(await readPosition()).not.toBe("fixed");
    await expect(heading).toBeVisible();

    // Rotate back to landscape (device turned wide again) — still native,
    // proving the behavior is a live, reversible response to the viewport, not
    // a stuck one-shot state.
    await page.setViewportSize({ width: 900, height: 500 });
    await expect.poll(readTransform).toBe("none");
    await expect(heading).toBeVisible();

    expect(consoleMessages).toEqual([]);
  });
});

// Desktop browsers report a FINE (mouse/trackpad) primary pointer regardless
// of window aspect ratio. A narrow/portrait-shaped desktop window must render
// natively (unrotated) — it always did (the removed lock rules were gated on
// `(pointer: coarse)`); this guards that the reversal did not disturb it.
test("desktop (fine pointer) with a portrait-shaped window renders natively (#699)", async ({
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
