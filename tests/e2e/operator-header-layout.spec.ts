/**
 * Operator header layout E2E (#875 H1, mockup v3).
 *
 * The desktop header is ONE row, at most 44 px high, at every desktop width:
 * Piesne, Biblia, the Stránky / Obrazovky / Pripojenia menus, the search (not
 * on a desktop also on Biblia, mockup v3), Stage Output, Live/Edit and ⚙. No control lies over another, nothing runs
 * off the right edge. The menus overlay the page (the header never grows),
 * only one is open at a time, and Escape, a press outside and picking an item
 * close them. A Resolume host that is down turns the closed Pripojenia button
 * orange with its name.
 *
 * Also asserts zero browser console errors/warnings per
 * ci/browser-console-zero-errors.md.
 */

import { test, expect, type Page } from "@playwright/test";
import {
  attachConsoleErrorCollector,
  deriveTestConfig,
  headerMenu,
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

const WIDTHS = [1024, 1280, 1366, 1440, 1600, 1920, 2560];

/* The controls of the header ROW: visible ones (a closed menu panel has no
   box). On Timers & co. the stage counter and broom are row controls too. Each
   evaluate below carries its own copy of this filter (no eval in the page). */
const ROW_SELECTOR =
  ".operator__header a, .operator__header button, .operator__header select, .operator__header input";

/** Pairs of header controls that overlap by more than a pixel each way. */
async function overlappingControls(page: Page): Promise<string[]> {
  return page.evaluate((selector) => {
    const controls = [...document.querySelectorAll<HTMLElement>(selector)].filter(
      (el) => el.getClientRects().length > 0,
    );
    const name = (el: HTMLElement) =>
      (el.innerText || el.getAttribute("aria-label") || el.getAttribute("placeholder") || el.tagName).trim();
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
  }, ROW_SELECTOR);
}

/** The header's height, the row controls lying outside it or the window, the
 *  number of row controls and the page width. */
async function rowFacts(page: Page) {
  return page.evaluate((selector) => {
    const header = document.querySelector(".operator__header")!.getBoundingClientRect();
    const controls = [...document.querySelectorAll<HTMLElement>(selector)].filter(
      (el) => el.getClientRects().length > 0,
    );
    const outside = controls
      .filter((el) => {
        const r = el.getBoundingClientRect();
        return r.top < header.top - 1 || r.bottom > header.bottom + 1 || r.right > window.innerWidth;
      })
      .map((el) => (el.innerText || el.getAttribute("aria-label") || el.tagName).trim());
    return {
      height: header.height,
      outside,
      controls: controls.length,
      pageWidth: document.documentElement.scrollWidth,
    };
  }, ROW_SELECTOR);
}

async function openOperator(page: Page, path: string) {
  await page.goto(`${baseURL}${path}`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached" });
  await expect(page.locator("body")).toHaveAttribute("data-shell", "desktop");
}

for (const path of ["/ui/operator", "/ui/bible", "/ui/operator/timers"]) {
  test(`one 40 px header row, nothing covered or cut off, on ${path}`, async ({ page }) => {
    const errors: string[] = [];
    attachConsoleErrorCollector(page, errors);
    await openOperator(page, path);
    await expect(headerMenu(page, "connections").toggle).toBeVisible();
    // The desktop song search is in the row on every section, Biblia too
    // (mockup v3); only a phone's Biblia drops it (#850 B2c).
    await expect(page.locator('[data-role="global-search-query"]')).toHaveCount(1);

    for (const width of WIDTHS) {
      await page.setViewportSize({ width, height: 900 });
      const where = `${path} at ${width} px`;
      await expect.poll(() => overlappingControls(page), { message: where }).toEqual([]);
      const facts = await rowFacts(page);
      expect(facts.height, where).toBeLessThanOrEqual(44);
      expect(facts.outside, where).toEqual([]);
      // Piesne, Biblia, three menus, the search, a select, Live, Edit, ⚙.
      expect(facts.controls, where).toBeGreaterThanOrEqual(10);
      expect(facts.pageWidth, where).toBeLessThanOrEqual(width);
    }

    expect(errors).toEqual([]);
  });
}

test("menus open one at a time, overlay the page and close on Escape, outside press and pick", async ({
  page,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.setViewportSize({ width: 1280, height: 800 });
  await openOperator(page, "/ui/operator");
  const header = page.locator(".operator__header");
  const pages = headerMenu(page, "pages");
  const screens = headerMenu(page, "screens");
  const connections = headerMenu(page, "connections");
  const menus = [pages, screens, connections];

  for (const menu of menus) {
    await expect(menu.panel).toBeHidden();
    await expect(menu.toggle).toHaveAttribute("aria-expanded", "false");
  }

  // Each opens over the page, under the header, which keeps its height.
  for (const menu of menus) {
    await menu.toggle.click();
    await expect(menu.panel).toBeVisible();
    await expect(menu.toggle).toHaveAttribute("aria-expanded", "true");
    for (const other of menus.filter((m) => m !== menu)) {
      await expect(other.panel).toBeHidden();
    }
    const headerBox = (await header.boundingBox())!;
    const panelBox = (await menu.panel.boundingBox())!;
    expect(headerBox.height).toBeLessThanOrEqual(44);
    expect(panelBox.y).toBeGreaterThanOrEqual(headerBox.y + headerBox.height);
    expect(panelBox.x + panelBox.width).toBeLessThanOrEqual(1280);
  }
  // The same button again closes its menu.
  await connections.toggle.click();
  await expect(connections.panel).toBeHidden();

  // Escape closes and gives the focus back to the button.
  await pages.toggle.click();
  await expect(pages.panel).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(pages.panel).toBeHidden();
  await expect(pages.toggle).toBeFocused();

  // Keyboard: Enter opens, Tab reaches the first item.
  await page.keyboard.press("Enter");
  await expect(pages.panel).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(pages.panel.locator('[data-role="header-menu-study"]')).toBeFocused();
  await expect(pages.panel.locator('[data-role="header-menu-study"]')).toHaveAttribute(
    "href",
    "/ui/study",
  );

  // A press outside every menu (here: the search field) closes it.
  await page.locator('[data-role="global-search-query"]').click();
  await expect(pages.panel).toBeHidden();

  // The Stránky items, in mockup order; picking one opens it and closes the menu.
  await pages.toggle.click();
  await expect(pages.panel.locator(".header-menu__item")).toHaveText([
    "Štúdium",
    "Časovače",
    "AI",
    "Menovky",
  ]);
  await pages.panel.locator('[data-role="view-toggle"][data-view="timers"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-view", "timers");
  await expect(pages.panel).toBeHidden();
  await expect(pages.toggle).toHaveAttribute("data-active", "true");

  // The helper the other specs use, through the menu and through ⚙.
  await openView(page, "menovky");
  await expect(page.locator("body")).toHaveAttribute("data-view", "menovky");
  await openView(page, "settings");
  await expect(page.locator("body")).toHaveAttribute("data-view", "settings");
  await expect(page.locator('.operator__settings-button')).toHaveAttribute("data-active", "true");
  await expect(pages.toggle).toHaveAttribute("data-active", "false");
  expect(new URL(page.url()).pathname).toBe("/ui/operator/settings");
  await openView(page, "worship");
  await expect(page.locator("body")).toHaveAttribute("data-view", "worship");

  expect(errors).toEqual([]);
});

test("a Resolume host that is down turns the closed Pripojenia button orange with its name", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  // AI up: the button then speaks only about the Resolume host.
  await page.route("**/ai/status", (route) =>
    route.fulfill({ json: { connected: true, error: null, modelValid: true } }),
  );
  const label = "E2E wall down";
  // Nothing listens on port 1: the host goes to `error` on its first attempt.
  const created = await request.post(new URL("/integrations/resolume/hosts", baseURL).toString(), {
    data: { label, host: "127.0.0.1", port: 1, isEnabled: true },
  });
  expect(created.ok()).toBeTruthy();
  const { id } = (await created.json()) as { id: string };
  try {
    await page.setViewportSize({ width: 1280, height: 800 });
    await openOperator(page, "/ui/operator");
    const connections = headerMenu(page, "connections");

    await expect(connections.toggle).toHaveAttribute("data-state", "down", { timeout: 30_000 });
    await expect(connections.panel).toBeHidden();
    await expect(connections.toggle).toContainText(`Odpojené: ${label}`);
    const border = await connections.toggle.evaluate((el) => getComputedStyle(el).borderTopColor);
    expect(border).toBe("rgb(245, 158, 11)");
    // Still one row, nothing covered, with the longer label.
    expect((await rowFacts(page)).height).toBeLessThanOrEqual(44);
    await expect.poll(() => overlappingControls(page)).toEqual([]);

    // The list names the same host.
    await connections.toggle.click();
    await expect(
      connections.panel.locator(`[data-role="resolume-status-chip"][data-host-label="${label}"]`),
    ).toBeVisible();
  } finally {
    await request.delete(new URL(`/integrations/resolume/hosts/${id}`, baseURL).toString());
  }

  expect(errors).toEqual([]);
});
