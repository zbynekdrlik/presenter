/**
 * Worship operator slides rendering tests (#215).
 *
 * Verifies the worship slides page displays cleanly:
 * - No phantom stage-control__slide class
 * - Group badges are inside slide cards (not floating outside)
 * - No inline .operator__slide-group-label elements
 * - Zero console errors/warnings
 * - Repeated groups render with the --inherited modifier
 */

import { test, expect, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 120_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

type SelectionTarget = {
  libraryId: string;
  libraryName: string;
  presentationId: string;
  presentationName: string;
  hasGroups: boolean;
  hasRepeatedGroups: boolean;
};

/**
 * Queries the server's /libraries endpoint directly (via Playwright's fetch,
 * not in-page) and picks a worship presentation that exercises the rendering
 * paths we care about. Returns `null` if the corpus has no suitable fixture —
 * callers MUST fail loudly rather than silently pass.
 */
async function pickWorshipTarget(
  page: Page,
  { requiresRepeatedGroups }: { requiresRepeatedGroups: boolean },
): Promise<SelectionTarget | null> {
  const response = await page.request.get(`${baseURL}/libraries`);
  expect(response.ok()).toBe(true);
  const libs = (await response.json()) as any[];

  type Candidate = SelectionTarget & { size: number };
  let best: Candidate | null = null;

  for (const lib of libs) {
    for (const p of lib.presentations ?? []) {
      const slides = p.slides ?? [];
      if (slides.length === 0) continue;
      const groups = slides
        .map((s: any) => s.content?.group?.name as string | undefined)
        .filter((g: string | undefined): g is string => !!g);
      const hasGroups = groups.length > 0;
      const hasRepeatedGroups = new Set(groups).size < groups.length;
      if (requiresRepeatedGroups && !hasRepeatedGroups) continue;
      if (!requiresRepeatedGroups && !hasGroups) continue;
      const candidate: Candidate = {
        libraryId: lib.id,
        libraryName: lib.name,
        presentationId: p.id,
        presentationName: p.name,
        hasGroups,
        hasRepeatedGroups,
        size: slides.length,
      };
      if (!best || candidate.size > best.size) {
        best = candidate;
      }
    }
  }

  if (!best) return null;
  const { size: _size, ...target } = best;
  return target;
}

/**
 * Opens the operator UI, navigates to a specific library + presentation by id.
 * Uses DOM clicks on data attributes rather than text matching.
 */
async function openPresentation(page: Page, target: SelectionTarget) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Click the specific library card by id.
  const libClicked = await page.evaluate((libId: string) => {
    const cards = document.querySelectorAll<HTMLElement>(
      `[data-role="library-list"] [data-library-id="${libId}"], [data-library-id="${libId}"]`,
    );
    if (cards.length === 0) return false;
    const btn =
      (cards[0].querySelector("button") as HTMLElement | null) ?? cards[0];
    btn.click();
    return true;
  }, target.libraryId);
  expect(libClicked, `library ${target.libraryName} not found in UI`).toBe(
    true,
  );

  // Generous timeout: largest libraries have ~500 presentations, and Leptos
  // renders the whole For block synchronously before the first item is in
  // the DOM. 10s wasn't enough for LIVING STONES (495).
  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 30_000,
  });

  // Click the specific presentation by id.
  const presClicked = await page.evaluate((presId: string) => {
    const item = document.querySelector<HTMLElement>(
      `[data-role="presentation-item"][data-presentation-id="${presId}"]`,
    );
    if (!item) return false;
    item.scrollIntoView({ block: "center" });
    const btn =
      (item.querySelector("button, [role='button']") as HTMLElement | null) ??
      item;
    btn.click();
    return true;
  }, target.presentationId);
  expect(
    presClicked,
    `presentation ${target.presentationName} not found in UI`,
  ).toBe(true);

  await page.waitForSelector("[data-slide-id]", { timeout: 10_000 });
}

test("worship slides render without phantom class or outside-card groups", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Just need a worship presentation with any slides + groups — doesn't
  // have to have repeated groups for this test.
  const target = await pickWorshipTarget(page, {
    requiresRepeatedGroups: false,
  });
  expect(
    target,
    "test corpus has no worship presentations with groups — fixtures broken",
  ).not.toBeNull();

  await openPresentation(page, target!);

  // No element has the phantom "stage-control__slide" class
  const phantomCount = await page.locator(".stage-control__slide").count();
  expect(phantomCount).toBe(0);

  // All slide cards have the worship variant class
  const worshipCards = await page
    .locator(".operator__slide-card--worship")
    .count();
  expect(worshipCards).toBeGreaterThan(0);

  // All slide-group elements are INSIDE a slide card (no orphans as siblings)
  const orphanGroups = await page
    .locator('[data-role="slide-group"]')
    .evaluateAll(
      (elements) =>
        elements.filter((el) => !el.closest(".operator__slide-card")).length,
    );
  expect(orphanGroups).toBe(0);

  // At least one group badge is rendered (the chosen presentation has groups).
  const badges = await page.locator('[data-role="slide-group"]').count();
  expect(badges).toBeGreaterThan(0);

  // No inline .operator__slide-group-label (removed in #215)
  const inlineLabels = await page
    .locator(".operator__slide-group-label")
    .count();
  expect(inlineLabels).toBe(0);

  // Clean console
  expect(consoleMessages).toEqual([]);
});

test("worship slides use --inherited modifier for repeated groups", async ({
  page,
}) => {
  // This test specifically asserts inherited-group rendering, so it MUST
  // open a presentation that has at least one repeated group. Fail loudly
  // if no such fixture exists.
  const target = await pickWorshipTarget(page, {
    requiresRepeatedGroups: true,
  });
  expect(
    target,
    "test corpus has no worship presentations with repeated groups — cannot assert inherited modifier",
  ).not.toBeNull();

  await openPresentation(page, target!);

  // At least one inherited badge must be in the DOM, since by definition
  // a repeated group means the 2nd+ occurrence is inherited.
  const inheritedCount = await page
    .locator(".operator__slide-group--inherited")
    .count();
  expect(inheritedCount).toBeGreaterThan(0);

  // And at least one NON-inherited badge (the first occurrence).
  const nonInheritedCount = await page
    .locator(".operator__slide-group:not(.operator__slide-group--inherited)")
    .count();
  expect(nonInheritedCount).toBeGreaterThan(0);
});
