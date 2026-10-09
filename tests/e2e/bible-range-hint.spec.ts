/**
 * #825 — Bible page: a chapter or verse past the selected book's range is
 * clamped to the last one, the box shows the maximum ("/ N", `max=`), and an
 * inline note explains it ("Kniha má len N kapitol" / "Kapitola má len M
 * veršov"). The #257 keyboard flow keeps moving (chapter → Enter → verse
 * start → Enter → verse end), the #702 start→end mirror stays, and the note
 * clears on the next valid value or a book change.
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

/** Chapter count of 1 Ján in SEB and the verse count of its last chapter. */
async function firstJohnCounts(): Promise<{ chapters: number; lastVerses: number }> {
  const response = await fetch(
    new URL("/bible/books?translation=slk-seb", baseURL),
  );
  expect(response.ok).toBe(true);
  const books = (await response.json()) as Array<{
    code: string;
    chapters: Array<{ number: number; verseCount: number }>;
  }>;
  const book = books.find((candidate) => candidate.code === "1JN");
  expect(book).toBeDefined();
  const chapters = book!.chapters.length;
  return { chapters, lastVerses: book!.chapters[chapters - 1].verseCount };
}

test("an over-range chapter and verse are clamped with an inline hint; the keyboard flow keeps moving", async ({
  page,
}) => {
  const consoleMessages = collectConsoleMessages(page);
  const { chapters, lastVerses } = await firstJohnCounts();

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
  await expect(page.locator('[data-role="book-list"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  await page.locator('[data-role="book-item"][data-book-code="1JN"]').click();

  const chapter = page.locator('[data-role="chapter-input"]');
  const verseStart = page.locator('[data-role="verse-start"]');
  const verseEnd = page.locator('[data-role="verse-end"]');
  const hint = page.locator('[data-role="bible-range-hint"]');

  // The maximum is shown right at the box.
  await expect(chapter).toHaveAttribute("max", String(chapters));
  await expect(page.locator('[data-role="chapter-max"]')).toHaveText(
    `/ ${chapters}`,
  );

  // Chapter 60 → clamped to the last chapter, explained; Enter still moves on.
  await chapter.fill("60");
  await chapter.press("Enter");
  await expect(chapter).toHaveValue(String(chapters));
  await expect(hint).toHaveText(`Kniha má len ${chapters} kapitol`);
  await expect(verseStart).toBeFocused();
  await expect(verseStart).toHaveAttribute("max", String(lastVerses));
  await expect(page.locator('[data-role="verse-max"]')).toHaveText(
    `/ ${lastVerses}`,
  );

  // Verse 99 → clamped to the chapter's last verse, explained; the end mirrors.
  await verseStart.fill("99");
  await expect(verseStart).toHaveValue(String(lastVerses));
  await expect(hint).toHaveText(`Kapitola má len ${lastVerses} veršov`);
  await verseStart.press("Enter");
  await expect(verseEnd).toBeFocused();
  await expect(verseEnd).toHaveValue(String(lastVerses));
  // Enter re-commits the clamped value: the note must survive it.
  await expect(hint).toHaveText(`Kapitola má len ${lastVerses} veršov`);

  // The next valid value clears the note.
  await verseStart.fill("2");
  await expect(verseStart).toHaveValue("2");
  await expect(verseEnd).toHaveValue("2");
  await expect(hint).toHaveCount(0);

  // An over-range verse END is clamped too — on Tab and on Enter (Enter
  // blurs the box, which re-commits the clamped value).
  await verseEnd.fill("99");
  await verseEnd.press("Tab");
  await expect(verseEnd).toHaveValue(String(lastVerses));
  await expect(hint).toHaveText(`Kapitola má len ${lastVerses} veršov`);
  await verseStart.fill("2");
  await expect(hint).toHaveCount(0);
  await verseEnd.fill("99");
  await verseEnd.press("Enter");
  await expect(verseEnd).toHaveValue(String(lastVerses));
  await expect(hint).toHaveText(`Kapitola má len ${lastVerses} veršov`);

  // A book change clears the note.
  await page.locator('[data-role="book-filter"]').fill("Gen");
  await page.locator('[data-role="book-item"][data-book-code="GEN"]').click();
  await expect(hint).toHaveCount(0);

  expect(consoleMessages).toEqual([]);
});
