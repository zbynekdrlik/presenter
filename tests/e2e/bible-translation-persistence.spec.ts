/**
 * #701 — Bible live: remember the last selected translations PER BROWSER.
 *
 * The primary + secondary translation selection must persist across a refresh,
 * per browser (localStorage), NOT as a server-global preference — several
 * operators work at the same time on different machines. An explicitly EMPTY
 * secondary is a valid remembered state (distinct from "never chosen").
 *
 * RED before the fix: selection is persisted only via the server-global
 * `bible-preferences` row, so two browsers cannot keep independent selections —
 * whichever wrote last wins for both, and one operator clearing the secondary is
 * clobbered by another operator's saved secondary after refresh.
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

/** Open a fresh page on the bible tab with translations loaded. */
async function openBible(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  // Main translation auto-selects once the list loads.
  await expect(page.locator('[data-role="main-translation"]')).toHaveValue(
    /.+/,
    { timeout: 15_000 },
  );
}

async function openBibleContext(
  browser: Browser,
): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await openBible(page);
  return { page, close: () => context.close() };
}

test("primary + secondary translation selection persists across refresh", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
  });

  await openBible(page);

  const main = page.locator('[data-role="main-translation"]');
  const secondary = page.locator('[data-role="secondary-translation"]');

  // Need a non-default main (>=2 translations) and >=2 secondary options besides
  // "None" for a meaningful test.
  if (
    (await main.locator("option").count()) < 2 ||
    (await secondary.locator("option").count()) < 3
  ) {
    test.skip(true, "Need >=2 bible translations for the persistence test");
    return;
  }

  // Pick a NON-default main (index 1 — the default is index 0) so the assertion
  // actually fails if persistence is broken, plus a concrete non-empty secondary.
  await main.selectOption({ index: 1 });
  await secondary.selectOption({ index: 2 }); // index 0 == "None"
  const chosenMain = await main.inputValue();
  const chosenSecondary = await secondary.inputValue();
  expect(chosenSecondary).not.toBe("");

  // Let the persistence write settle, then reload.
  await expect(main).toHaveValue(chosenMain);
  await expect(secondary).toHaveValue(chosenSecondary);
  await page.waitForTimeout(500);
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  await expect(page.locator('[data-role="main-translation"]')).toHaveValue(
    chosenMain,
    { timeout: 15_000 },
  );
  await expect(page.locator('[data-role="secondary-translation"]')).toHaveValue(
    chosenSecondary,
  );

  expect(consoleMessages).toEqual([]);
});

test("explicitly-empty secondary persists and browsers keep independent selections", async ({
  browser,
}) => {
  const a = await openBibleContext(browser);
  const b = await openBibleContext(browser);
  try {
    const aMain = a.page.locator('[data-role="main-translation"]');
    const aSecondary = a.page.locator('[data-role="secondary-translation"]');
    const bSecondary = b.page.locator('[data-role="secondary-translation"]');

    // Need >=2 secondary options besides "None" for a concrete non-empty pick.
    if ((await aSecondary.locator("option").count()) < 3) {
      test.skip(true, "Need >=2 secondary bible translations for this test");
      return;
    }

    // Browser A: choose a concrete, non-empty secondary.
    await aMain.selectOption({ index: 0 });
    await aSecondary.selectOption({ index: 2 });
    const aChosenMain = await aMain.inputValue();
    const aChosenSecondary = await aSecondary.inputValue();
    expect(aChosenSecondary).not.toBe("");
    await expect(aSecondary).toHaveValue(aChosenSecondary);

    // Browser B: explicitly CLEAR the secondary (select "None").
    await bSecondary.selectOption({ index: 0 });
    await expect(bSecondary).toHaveValue("");

    // Let both persistence writes settle, then reload both.
    await a.page.waitForTimeout(500);
    await b.page.waitForTimeout(500);
    await a.page.reload({ waitUntil: "domcontentloaded" });
    await b.page.reload({ waitUntil: "domcontentloaded" });
    await a.page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await b.page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    // Browser A keeps ITS chosen secondary (not clobbered by B's clear).
    await expect(
      a.page.locator('[data-role="main-translation"]'),
    ).toHaveValue(aChosenMain, { timeout: 15_000 });
    await expect(
      a.page.locator('[data-role="secondary-translation"]'),
    ).toHaveValue(aChosenSecondary);

    // Browser B stays explicitly empty (explicitly-cleared, not re-defaulted).
    await expect(
      b.page.locator('[data-role="secondary-translation"]'),
    ).toHaveValue("", { timeout: 15_000 });
  } finally {
    await a.close();
    await b.close();
  }
});
