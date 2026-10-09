/**
 * #828 — Bible slide cards show a small character count for the main text
 * and the translation text, warning-coloured (`data-over="true"`) when the
 * count is over the Bible character limit.
 *
 * Like an operator: SEB main + KJV secondary, load 1 Ján 1:1-3. With a high
 * limit every badge equals the characters of its text and is not "over";
 * with a 50-character limit each verse is longer than the limit on its own,
 * so the badges flag it.
 */

import { test, expect, type Locator, type Page } from "@playwright/test";
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

async function setCharacterLimit(limit: number): Promise<void> {
  const response = await fetch(new URL("/bible/preferences", baseURL), {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ characterLimit: limit }),
  });
  expect(response.ok).toBe(true);
}

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

/** Open the Bible page, pick SEB + KJV and load 1 Ján 1:1-3 by button. */
async function loadFirstJohn(page: Page): Promise<void> {
  await page.goto(new URL("/ui/operator/bible", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  const main = page.locator('[data-role="main-translation"]');
  await expect(main.locator('option[value="slk-seb"]')).toHaveCount(1, {
    timeout: 15_000,
  });
  await main.selectOption("slk-seb");
  await page
    .locator('[data-role="secondary-translation"]')
    .selectOption("eng-kjv");
  await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  await page.locator('[data-role="book-item"][data-book-code="1JN"]').click();
  const verseStart = page.locator('[data-role="verse-start"]');
  await verseStart.fill("1");
  await verseStart.press("Tab");
  const verseEnd = page.locator('[data-role="verse-end"]');
  await verseEnd.fill("3");
  await verseEnd.press("Tab");
  await page.locator('[data-role="load-button"]').click();
  await expect(
    page.locator('[data-role="slide-card"]').first(),
  ).toBeVisible({ timeout: 15_000 });
}

/** Characters (Unicode scalar values, like Rust `chars().count()`). */
async function charCount(text: Locator): Promise<number> {
  const value = (await text.textContent()) ?? "";
  return [...value].length;
}

test("each slide card shows the character count of its main and translation text", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  await setCharacterLimit(2000);
  await loadFirstJohn(page);

  const card = page.locator('[data-role="slide-card"]').first();
  await expect(
    card.locator(".operator__slide-reference--secondary"),
  ).toHaveText(/1:1-3 \(KJV\)$/);
  const mainBadge = card.locator(
    '[data-role="slide-char-count"][data-field="main"]',
  );
  const translationBadge = card.locator(
    '[data-role="slide-char-count"][data-field="translation"]',
  );
  const mainChars = await charCount(card.locator(".operator__slide-text--main"));
  const translationChars = await charCount(
    card.locator(".operator__slide-text--translation"),
  );
  expect(mainChars).toBeGreaterThan(0);
  expect(translationChars).toBeGreaterThan(0);
  await expect(mainBadge).toHaveText(String(mainChars));
  await expect(translationBadge).toHaveText(String(translationChars));
  await expect(mainBadge).toHaveAttribute("data-over", "false");
  await expect(translationBadge).toHaveAttribute("data-over", "false");

  expect(consoleMessages).toEqual([]);
});

test("a count over the character limit is flagged", async ({ page }) => {
  const consoleMessages = collectConsoleMessages(page);
  // Every verse of 1 John 1:1-3 is longer than 50 characters on its own, so
  // each one stays a whole slide (never cut) and is over the limit.
  await setCharacterLimit(50);
  await loadFirstJohn(page);

  const cards = page.locator('[data-role="slide-card"]');
  await expect(cards).toHaveCount(3);
  const card = cards.first();
  const mainBadge = card.locator(
    '[data-role="slide-char-count"][data-field="main"]',
  );
  await expect(mainBadge).toHaveAttribute("data-over", "true");
  await expect(
    card.locator('[data-role="slide-char-count"][data-field="translation"]'),
  ).toHaveAttribute("data-over", "true");
  expect(Number(await mainBadge.textContent())).toBeGreaterThan(50);

  expect(consoleMessages).toEqual([]);
});
