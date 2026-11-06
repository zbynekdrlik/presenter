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

/** A button of the verse console's translation bar (#850 B2c replaced the selects). */
function button(page: Page, kind: "main" | "secondary", code: string) {
  return page.locator(
    `[data-role="console-translation"][data-kind="${kind}"][data-translation="${code}"]`,
  );
}

/** The code of the pressed button of a row ("" = žiadny). */
function pressed(page: Page, kind: "main" | "secondary") {
  return page
    .locator(`[data-role="console-translation"][data-kind="${kind}"][aria-pressed="true"]`)
    .getAttribute("data-translation");
}

/** Open a fresh page on the bible tab with translations loaded. */
async function openBible(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  // Main translation auto-selects once the list loads.
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    /.+/,
    { timeout: 15_000 },
  );
}

/** Wait until this browser remembers `code` for a row (#701 localStorage). */
async function remembered(page: Page, kind: "main" | "secondary", code: string) {
  const key = kind === "main" ? "presenter:bibleMainTranslation" : "presenter:bibleSecondaryTranslation";
  await expect
    .poll(() => page.evaluate((k) => localStorage.getItem(k), key))
    .toBe(JSON.stringify(code));
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

  // A main that is NOT on now, so the assertion fails if persistence is
  // broken, plus a concrete non-empty secondary.
  const before = await pressed(page, "main");
  const chosenMain = before === "slk-roh" ? "slk-sevp" : "slk-roh";
  await button(page, "main", chosenMain).click();
  await remembered(page, "main", chosenMain);
  await button(page, "secondary", "eng-kjv").click();
  await remembered(page, "secondary", "eng-kjv");
  await expect(button(page, "main", chosenMain)).toHaveAttribute("aria-pressed", "true");
  await expect(button(page, "secondary", "eng-kjv")).toHaveAttribute("aria-pressed", "true");

  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    chosenMain,
    { timeout: 15_000 },
  );
  await expect(button(page, "main", chosenMain)).toHaveAttribute("aria-pressed", "true");
  await expect(button(page, "secondary", "eng-kjv")).toHaveAttribute("aria-pressed", "true");

  expect(consoleMessages).toEqual([]);
});

test("explicitly-empty secondary persists and browsers keep independent selections", async ({
  browser,
}) => {
  const a = await openBibleContext(browser);
  const b = await openBibleContext(browser);
  try {
    // Browser A: SEB with a concrete, non-empty secondary.
    await button(a.page, "main", "slk-seb").click();
    await remembered(a.page, "main", "slk-seb");
    await button(a.page, "secondary", "eng-kjv").click();
    await remembered(a.page, "secondary", "eng-kjv");
    await expect(button(a.page, "secondary", "eng-kjv")).toHaveAttribute("aria-pressed", "true");

    // Browser B: explicitly CLEAR the secondary ("žiadny").
    await button(b.page, "secondary", "").click();
    await remembered(b.page, "secondary", "");
    await expect(button(b.page, "secondary", "")).toHaveAttribute("aria-pressed", "true");

    await a.page.reload({ waitUntil: "domcontentloaded" });
    await b.page.reload({ waitUntil: "domcontentloaded" });
    await a.page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await b.page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    // Browser A keeps ITS chosen secondary (not clobbered by B's clear).
    await expect(a.page.locator('[data-role="bible-console"]')).toHaveAttribute(
      "data-books-translation",
      "slk-seb",
      { timeout: 15_000 },
    );
    await expect(button(a.page, "main", "slk-seb")).toHaveAttribute("aria-pressed", "true");
    await expect(button(a.page, "secondary", "eng-kjv")).toHaveAttribute("aria-pressed", "true");

    // Browser B stays explicitly empty (explicitly-cleared, not re-defaulted).
    await expect(button(b.page, "secondary", "")).toHaveAttribute("aria-pressed", "true", {
      timeout: 15_000,
    });
  } finally {
    await a.close();
    await b.close();
  }
});
