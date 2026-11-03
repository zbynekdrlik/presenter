/**
 * Operator header layout E2E (#875).
 *
 * No control in the header may lie over another one, at any desktop width and
 * on any operator view. The surface links used to be positioned out of flow,
 * so on a view with a short header the view tabs rose into them.
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
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

const WIDTHS = [1024, 1280, 1366, 1440, 1600, 1920];

/** Pairs of header controls that overlap by more than a pixel each way. */
async function overlappingControls(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const controls = [
      ...document.querySelectorAll<HTMLElement>(
        ".operator__header a, .operator__header button, .operator__header select, .operator__header input",
      ),
    ].filter((el) => el.getClientRects().length > 0);
    const name = (el: HTMLElement) =>
      (el.innerText || el.getAttribute("placeholder") || el.tagName).trim();
    const pairs: string[] = [];
    controls.forEach((a, i) => {
      controls.slice(i + 1).forEach((b) => {
        if (a.contains(b) || b.contains(a)) return;
        const r = a.getBoundingClientRect();
        const s = b.getBoundingClientRect();
        const x = Math.min(r.right, s.right) - Math.max(r.left, s.left);
        const y = Math.min(r.bottom, s.bottom) - Math.max(r.top, s.top);
        if (x > 1 && y > 1) pairs.push(`${name(a)} × ${name(b)}`);
      });
    });
    return pairs;
  });
}

for (const path of ["/ui/operator", "/ui/bible"]) {
  test(`no header control covers another on ${path}`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await page.goto(`${baseURL}${path}`);
    await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
    await expect(page.locator(".operator__surface-nav")).toBeVisible();

    for (const width of WIDTHS) {
      await page.setViewportSize({ width, height: 900 });
      await expect
        .poll(() => overlappingControls(page), { message: `${path} at ${width} px` })
        .toEqual([]);
      const pageWidth = await page.evaluate(() => document.documentElement.scrollWidth);
      expect(pageWidth, `${path} at ${width} px`).toBeLessThanOrEqual(width);
    }

    expect(errors).toEqual([]);
  });
}
