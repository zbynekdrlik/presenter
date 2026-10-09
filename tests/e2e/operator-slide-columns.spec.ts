/**
 * #832 — Operator: per-browser "slides per row" (1–8), remembered in
 * localStorage, default 3. The control in the slides toolbar drives every
 * `.operator__slides` grid (worship + Bible live/prepared); an explicit choice
 * beats the ≤480 px phone default of 2; cards stay readable at 8 with no
 * horizontal overflow.
 */

import { test, expect, type Browser, type Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

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

function collectConsoleMessages(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    messages.push(`[${msg.type()}] ${msg.text()}`);
  });
  page.on("pageerror", (err) => {
    messages.push(`[pageerror] ${err.message}`);
  });
  return messages;
}

async function openOperator(page: Page, path = "/ui/operator"): Promise<void> {
  await page.goto(new URL(path, baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
}

/**
 * Column count of every VISIBLE `.operator__slides` grid: a rendered grid
 * reports its resolved tracks ("120px 120px …"); a hidden one only the
 * specified `repeat(…)` text, so hidden grids are skipped.
 */
async function gridColumns(page: Page): Promise<number[]> {
  return page.evaluate(() =>
    Array.from(document.querySelectorAll(".operator__slides"))
      .filter((grid) => (grid as HTMLElement).offsetParent !== null)
      .map(
        (grid) =>
          getComputedStyle(grid)
            .gridTemplateColumns.split(" ")
            .filter((track) => track.trim().length > 0).length,
      ),
  );
}

async function expectColumns(page: Page, columns: number): Promise<void> {
  await expect
    .poll(async () => {
      const counts = await gridColumns(page);
      return counts.length > 0 && counts.every((count) => count === columns);
    })
    .toBe(true);
}

/** The visible slides-per-row control of the current view. */
function control(page: Page) {
  return page.locator('[data-role="slide-columns-control"]:visible');
}

async function newContextPage(
  browser: Browser,
  viewport?: { width: number; height: number },
): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext(viewport ? { viewport } : {});
  const page = await context.newPage();
  return { page, close: () => context.close() };
}

test("slides per row: set 5, it survives a reload, a fresh browser keeps the default 3", async ({
  page,
  browser,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page);

  await expect(control(page).locator('[data-role="slide-columns-value"]')).toHaveText("3");
  await expectColumns(page, 3);

  const increase = control(page).locator('[data-role="slide-columns-increase"]');
  await increase.click();
  await increase.click();
  await expect(control(page).locator('[data-role="slide-columns-value"]')).toHaveText("5");
  await expectColumns(page, 5);

  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await expect(control(page).locator('[data-role="slide-columns-value"]')).toHaveText("5");
  await expectColumns(page, 5);

  // The Bible page's grids follow the same per-browser choice.
  await openOperator(page, "/ui/operator/bible");
  await expect(control(page).locator('[data-role="slide-columns-value"]')).toHaveText("5");
  await expectColumns(page, 5);

  const fresh = await newContextPage(browser);
  try {
    await openOperator(fresh.page);
    await expect(
      control(fresh.page).locator('[data-role="slide-columns-value"]'),
    ).toHaveText("3");
    await expectColumns(fresh.page, 3);
  } finally {
    await fresh.close();
  }

  expect(consoleMessages).toEqual([]);
});

test("a phone keeps 2 per row by default, an explicit choice wins", async ({
  browser,
}) => {
  const phone = await newContextPage(browser, { width: 400, height: 800 });
  const consoleMessages = collectConsoleMessages(phone.page);
  try {
    await openOperator(phone.page);
    await expectColumns(phone.page, 2);
    // The control shows what the phone grid shows.
    await expect(
      control(phone.page).locator('[data-role="slide-columns-value"]'),
    ).toHaveText("2");

    // An explicit choice made in this browser beats the phone default.
    await phone.page.setViewportSize({ width: 1280, height: 800 });
    await expect(
      control(phone.page).locator('[data-role="slide-columns-value"]'),
    ).toHaveText("3");
    await control(phone.page)
      .locator('[data-role="slide-columns-increase"]')
      .click();
    await expectColumns(phone.page, 4);
    await phone.page.setViewportSize({ width: 400, height: 800 });
    await expectColumns(phone.page, 4);
    expect(consoleMessages).toEqual([]);
  } finally {
    await phone.close();
  }
});

test("8 per row: Bible cards stay readable with no horizontal overflow", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await openOperator(page, "/ui/operator/bible");

  const decrease = control(page).locator('[data-role="slide-columns-decrease"]');
  const increase = control(page).locator('[data-role="slide-columns-increase"]');
  await decrease.click(); // never below 1: the stepper is bounded
  for (let i = 0; i < 10; i += 1) {
    await increase.click();
  }
  await expect(control(page).locator('[data-role="slide-columns-value"]')).toHaveText("8");

  const main = page.locator('[data-role="main-translation"]');
  await expect(main.locator('option[value="slk-seb"]')).toHaveCount(1, {
    timeout: 15_000,
  });
  await main.selectOption("slk-seb");
  await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  await page.locator('[data-role="book-item"][data-book-code="1JN"]').click();
  await page.locator('[data-role="load-button"]').click();
  const cards = page.locator('[data-role="slide-card"]');
  await expect(cards.first()).toBeVisible({ timeout: 15_000 });
  await expectColumns(page, 8);

  const overflow = await page.evaluate(() =>
    Array.from(document.querySelectorAll(".operator__slides"))
      .filter((grid) => (grid as HTMLElement).offsetParent !== null)
      .map((grid) => grid.scrollWidth - grid.clientWidth),
  );
  expect(overflow.length).toBeGreaterThan(0);
  expect(overflow.every((extra) => extra <= 1)).toBe(true);
  // A card's text is still on screen inside its card.
  const box = await cards.first().locator(".operator__slide-text--main").boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(40);

  expect(consoleMessages).toEqual([]);
});
