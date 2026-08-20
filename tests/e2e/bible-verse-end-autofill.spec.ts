/**
 * #702 — Bible: auto-fill verse END with the verse START value.
 *
 * Entering a verse start auto-fills the verse end with the same number (the
 * dominant case is triggering a single verse). The end stays fully editable:
 * typing a larger end keeps the range, and it is never overwritten until the
 * start changes again (then it re-mirrors).
 */

import { test, expect, type Page } from "@playwright/test";
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

async function openBible(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await expect(page.locator('[data-role="main-translation"]')).toHaveValue(
    /.+/,
    { timeout: 15_000 },
  );
}

async function hasBibleData(page: Page): Promise<boolean> {
  return (await page.locator('[data-role="book-item"]').count()) > 0;
}

test("verse end mirrors verse start; explicit end persists; start re-mirrors", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
  });

  await openBible(page);

  const verseStart = page.locator('[data-role="verse-start"]');
  const verseEnd = page.locator('[data-role="verse-end"]');

  // Enter a verse start -> the end auto-fills with the same number.
  await verseStart.fill("5");
  await verseStart.press("Tab");
  await expect(verseEnd).toHaveValue("5");

  // Type a LARGER end -> the range is kept (start unchanged, end honored).
  await verseEnd.fill("10");
  await verseEnd.press("Tab");
  await expect(verseStart).toHaveValue("5");
  await expect(verseEnd).toHaveValue("10");

  // Changing the start again RE-mirrors the end (single-verse default returns).
  await verseStart.fill("7");
  await verseStart.press("Tab");
  await expect(verseEnd).toHaveValue("7");

  expect(consoleMessages).toEqual([]);
});

test("single-verse fast path: typing only the start loads a single verse", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() !== "error" && msg.type() !== "warning") return;
    if (msg.text().includes("crbug.com/981419")) return;
    consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
  });

  await openBible(page);

  if (!(await hasBibleData(page))) {
    test.skip(true, "No Bible data available");
    return;
  }

  // Select a book, then type ONLY the verse start (end auto-fills).
  await page.locator('[data-role="book-item"]').first().click();
  const verseStart = page.locator('[data-role="verse-start"]');
  await verseStart.fill("1");
  await verseStart.press("Tab");
  await expect(page.locator('[data-role="verse-end"]')).toHaveValue("1");

  // Load the passage — a single verse resolves (end == start).
  await page.locator('[data-role="load-button"]').click();
  await page.waitForFunction(
    () => document.querySelectorAll('[data-role="slide-card"]').length > 0,
    { timeout: 15_000 },
  );
  expect(
    await page.locator('[data-role="slide-card"]').count(),
  ).toBeGreaterThan(0);

  expect(consoleMessages).toEqual([]);
});
