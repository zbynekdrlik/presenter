/**
 * Stage preview placement E2E (#875 H2, mockup v3).
 *
 * On a desktop the stage preview is the LAST block of the left column: on
 * Piesne the live mirror under the library / playlist / song lists, on Biblia
 * the verse on the wall under the Bible tabs. It is the column's full width at
 * 16:9 and ends at the column's bottom edge; the lists end above it. Ableton +
 * Follow (top-left), the broom (top-right) and the stage counter
 * (bottom-right) lie on the picture and take clicks. Every other section
 * keeps the counter and the broom in the header row. Exactly one copy is
 * mounted at any time.
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  openView,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

const VIEWPORTS = [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
];

type Box = { top: number; bottom: number; left: number; right: number; width: number; height: number };

/** The boxes of the column, the preview in it and the given parts. */
async function boxes(page: Page, panel: string, parts: string[]) {
  return page.evaluate(
    ({ panel, parts }) => {
      const box = (el: Element | null): Box | null => {
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, width: r.width, height: r.height };
      };
      const column = document.querySelector(`[data-view-panel="${panel}"] [data-role="catalog"]`);
      return {
        column: box(column),
        preview: box(column?.querySelector('[data-role="stage-status"]') ?? null),
        parts: Object.fromEntries(
          parts.map((role) => [role, box(column?.querySelector(`[data-role="${role}"]`) ?? null)]),
        ) as Record<string, Box | null>,
      };
    },
    { panel, parts },
  );
}

/** The element under the centre of each control is the control (or inside it). */
async function hitTargets(page: Page, roles: string[]): Promise<Record<string, boolean>> {
  return page.evaluate((roles) => {
    const result: Record<string, boolean> = {};
    for (const role of roles) {
      const el = document.querySelector(`[data-role="${role}"]`);
      if (!el) {
        result[role] = false;
        continue;
      }
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      result[role] = !!hit && el.contains(hit);
    }
    return result;
  }, roles);
}

function expectInside(inner: Box, outer: Box, what: string) {
  expect(inner.left, what).toBeGreaterThanOrEqual(outer.left - 1);
  expect(inner.right, what).toBeLessThanOrEqual(outer.right + 1);
  expect(inner.top, what).toBeGreaterThanOrEqual(outer.top - 1);
  expect(inner.bottom, what).toBeLessThanOrEqual(outer.bottom + 1);
}

/** Full width of the column, 16:9, flush with its bottom edge. */
function expectPinned(column: Box, preview: Box, where: string) {
  expect(Math.abs(preview.width - column.width), `${where}: width`).toBeLessThanOrEqual(2);
  expect(Math.abs(preview.left - column.left), `${where}: left`).toBeLessThanOrEqual(2);
  expect(Math.abs(preview.height - (preview.width * 9) / 16), `${where}: 16:9`).toBeLessThanOrEqual(2);
  expect(Math.abs(preview.bottom - column.bottom), `${where}: bottom`).toBeLessThanOrEqual(2);
}

async function open(page: Page, path: string) {
  await page.goto(`${baseURL}${path}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
}

for (const viewport of VIEWPORTS) {
  const size = `${viewport.width}×${viewport.height}`;

  test(`Piesne: the live mirror is the bottom of the lists column at ${size}`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.setViewportSize(viewport);
    await open(page, "/ui/operator");

    // One copy, in the column, with the mirror.
    await expect(page.locator('[data-role="stage-status"]')).toHaveCount(1);
    const column = page.locator('[data-view-panel="worship"] [data-role="catalog"]');
    await expect(column.locator('[data-role="stage-status"]')).toBeVisible();
    await expect(column.locator('[data-role="stage-preview-frame"]')).toBeVisible();
    await expect(page.locator('.operator__header [data-role="stage-status"]')).toHaveCount(0);

    const roles = ["catalog-top", "catalog-bottom", "stage-preview-frame", "ableset-enable", "ableset-follow", "clear-slide", "stage-monitor"];
    const b = await boxes(page, "worship", roles);
    const preview = b.preview!;
    expectPinned(b.column!, preview, size);
    // The lists end above it.
    expect(b.parts["catalog-top"]!.bottom, "library + playlist lists").toBeLessThanOrEqual(preview.top + 1);
    expect(b.parts["catalog-bottom"]!.bottom, "song list").toBeLessThanOrEqual(preview.top + 1);
    expect(b.parts["catalog-bottom"]!.height, "song list keeps rows").toBeGreaterThanOrEqual(100);
    // The mirror fills the block; the controls lie on it, each in its corner.
    const frame = b.parts["stage-preview-frame"]!;
    expect(Math.abs(frame.width - preview.width)).toBeLessThanOrEqual(2);
    expect(Math.abs(frame.height - preview.height)).toBeLessThanOrEqual(2);
    for (const role of ["ableset-enable", "ableset-follow", "clear-slide", "stage-monitor"]) {
      expectInside(b.parts[role]!, preview, role);
    }
    const midX = preview.left + preview.width / 2;
    const midY = preview.top + preview.height / 2;
    for (const role of ["ableset-enable", "ableset-follow"]) {
      expect(b.parts[role]!.right, `${role} left half`).toBeLessThanOrEqual(midX);
      expect(b.parts[role]!.bottom, `${role} top half`).toBeLessThanOrEqual(midY);
    }
    expect(b.parts["clear-slide"]!.left, "broom right half").toBeGreaterThanOrEqual(midX);
    expect(b.parts["clear-slide"]!.bottom, "broom top half").toBeLessThanOrEqual(midY);
    expect(b.parts["stage-monitor"]!.left, "counter right half").toBeGreaterThanOrEqual(midX);
    expect(b.parts["stage-monitor"]!.top, "counter bottom half").toBeGreaterThanOrEqual(midY);

    expect(await hitTargets(page, ["ableset-enable", "ableset-follow", "clear-slide", "stage-monitor"])).toEqual({
      "ableset-enable": true,
      "ableset-follow": true,
      "clear-slide": true,
      "stage-monitor": true,
    });
    // A real click reaches the counter (it resets the stage baseline, no request).
    await page.locator('[data-role="stage-monitor"]').click();

    expect(errors).toEqual([]);
  });

  test(`Biblia: the verse on the wall is the bottom of the left column at ${size}`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.setViewportSize(viewport);
    await open(page, "/ui/operator/bible");

    await expect(page.locator('[data-role="stage-status"]')).toHaveCount(1);
    const column = page.locator('[data-view-panel="bible"] [data-role="catalog"]');
    await expect(column.locator('[data-role="stage-status"]')).toBeVisible();
    await expect(column.locator('[data-role="bible-preview"]')).toBeVisible();
    await expect(page.locator('[data-role="stage-preview-frame"]')).toHaveCount(0);
    await expect(page.locator('[data-role="worship-preview"]')).toBeHidden();

    const b = await boxes(page, "bible", ["catalog-top", "bible-preview", "clear-slide", "stage-monitor"]);
    const preview = b.preview!;
    expectPinned(b.column!, preview, size);
    expect(b.parts["catalog-top"]!.bottom, "Bible tabs").toBeLessThanOrEqual(preview.top + 1);
    for (const role of ["bible-preview", "clear-slide", "stage-monitor"]) {
      expectInside(b.parts[role]!, preview, role);
    }
    expect(await hitTargets(page, ["clear-slide", "stage-monitor"])).toEqual({
      "clear-slide": true,
      "stage-monitor": true,
    });

    expect(errors).toEqual([]);
  });
}

test("other sections keep the counter and the broom in the header; one copy across view switches", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.setViewportSize({ width: 1280, height: 720 });
  await open(page, "/ui/operator");
  const copies = page.locator('[data-role="stage-status"]');
  const mirror = page.locator('[data-role="stage-preview-frame"]');

  for (const view of ["timers", "ai", "menovky", "settings"]) {
    await openView(page, view);
    await expect(page.locator("body")).toHaveAttribute("data-view", view);
    await expect(copies).toHaveCount(1);
    await expect(page.locator('.operator__header [data-role="stage-status"]')).toHaveCount(1);
    await expect(mirror).toHaveCount(0);
    await expect(page.locator('[data-role="clear-slide"]')).toBeVisible();
    await expect(page.locator('[data-role="stage-monitor"]')).toBeVisible();
    const header = (await page.locator(".operator__header").boundingBox())!;
    expect(header.height, view).toBeLessThanOrEqual(44);
    expect(await hitTargets(page, ["clear-slide", "stage-monitor"]), view).toEqual({
      "clear-slide": true,
      "stage-monitor": true,
    });
  }

  await openView(page, "bible");
  await expect(copies).toHaveCount(1);
  await expect(page.locator('[data-view-panel="bible"] [data-role="stage-status"]')).toHaveCount(1);
  await expect(mirror).toHaveCount(0);

  await openView(page, "worship");
  await expect(copies).toHaveCount(1);
  await expect(page.locator('[data-view-panel="worship"] [data-role="stage-status"]')).toHaveCount(1);
  await expect(mirror).toHaveCount(1);

  expect(errors).toEqual([]);
});
