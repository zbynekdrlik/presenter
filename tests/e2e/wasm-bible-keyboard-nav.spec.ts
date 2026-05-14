/**
 * Bible keyboard navigation + diacritic-insensitive book search (#257).
 *
 * Scenarios:
 *   A. Typing ASCII "lukas" surfaces the Slovak "Lukáš" book.
 *   B. Enter steps focus through book-filter → chapter → verse-start →
 *      verse-end → book-filter (cleared) for a no-mouse passage workflow.
 */

import { test, expect } from "@playwright/test";
import {
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

/** Open the operator, switch to bible view, ensure WASM is ready. */
async function openBibleLive(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/ui/operator`);
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  const bibleToggle = page.locator(
    '[data-role="view-toggle"][data-view="bible"]',
  );
  await bibleToggle.click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-view") === "bible",
    { timeout: 5_000 },
  );
}

/** Pick a translation that contains diacritics in book names (SEB / Slovak). */
async function selectSlovakTranslation(
  page: import("@playwright/test").Page,
): Promise<boolean> {
  const mainSelect = page.locator('[data-role="main-translation"]');
  await expect(mainSelect).toBeVisible({ timeout: 10_000 });

  // Find the option whose label looks Slovak (SEB / Roháček / Milost / SEVP).
  const slovakValue = await mainSelect.evaluate((el) => {
    const select = el as HTMLSelectElement;
    for (const opt of Array.from(select.options)) {
      const label = opt.textContent ?? "";
      if (/SEB|Slovak|slovenský|Roh[áa]ček|Milost|SEVP/i.test(label)) {
        return opt.value;
      }
    }
    return null;
  });
  if (!slovakValue) {
    return false;
  }
  await mainSelect.selectOption(slovakValue);

  // Wait for the book list to repopulate against the new translation.
  await page.waitForFunction(
    () => document.querySelectorAll('[data-role="book-item"]').length > 0,
    { timeout: 15_000 },
  );
  return true;
}

test.describe("Bible keyboard nav + diacritic search (#257)", () => {
  test("ASCII filter 'lukas' surfaces 'Lukáš'", async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await openBibleLive(page);
    const haveSlovak = await selectSlovakTranslation(page);
    test.skip(!haveSlovak, "No diacritic-bearing translation available");

    const bookFilter = page.locator('[data-role="book-filter"]');
    await bookFilter.fill("lukas");

    // Top filtered item should be the Slovak Lukáš book.
    const firstItem = page.locator('[data-role="book-item"]').first();
    await expect(firstItem).toBeVisible({ timeout: 5_000 });
    await expect(firstItem).toContainText("Luk", { timeout: 5_000 });
    const label = (await firstItem.textContent()) ?? "";
    expect(label).toMatch(/Luk[aá][sš]/);

    expect(consoleErrors.filter((m) => !m.includes("favicon"))).toEqual([]);
  });

  test("Enter chain: book-filter → chapter → verse-start → verse-end → book-filter", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await openBibleLive(page);
    const haveSlovak = await selectSlovakTranslation(page);
    test.skip(!haveSlovak, "No diacritic-bearing translation available");

    // Step 0 — on mount, book-filter should already be focused.
    const activeRoleAtMount = await page.evaluate(
      () => document.activeElement?.getAttribute("data-role") ?? null,
    );
    expect(activeRoleAtMount).toBe("book-filter");

    // Step 1 — type "lukas" then Enter → focus moves to chapter.
    const bookFilter = page.locator('[data-role="book-filter"]');
    await bookFilter.fill("lukas");
    await page.waitForFunction(
      () => document.querySelectorAll('[data-role="book-item"]').length > 0,
      { timeout: 5_000 },
    );
    await bookFilter.press("Enter");

    await expect
      .poll(
        async () =>
          page.evaluate(
            () => document.activeElement?.getAttribute("data-role") ?? null,
          ),
        { timeout: 5_000 },
      )
      .toBe("chapter-input");

    // Step 2 — type "6" then Enter → focus moves to verse-start.
    const chapter = page.locator('[data-role="chapter-input"]');
    await chapter.fill("6");
    await chapter.press("Enter");
    await expect
      .poll(
        async () =>
          page.evaluate(
            () => document.activeElement?.getAttribute("data-role") ?? null,
          ),
        { timeout: 5_000 },
      )
      .toBe("verse-start");

    // Step 3 — type "38" then Enter → focus moves to verse-end.
    const verseStart = page.locator('[data-role="verse-start"]');
    await verseStart.fill("38");
    await verseStart.press("Enter");
    await expect
      .poll(
        async () =>
          page.evaluate(
            () => document.activeElement?.getAttribute("data-role") ?? null,
          ),
        { timeout: 5_000 },
      )
      .toBe("verse-end");

    // Step 4 — Enter on empty verse-end → focus returns to book-filter,
    // and book-filter value is cleared.
    const verseEnd = page.locator('[data-role="verse-end"]');
    await verseEnd.press("Enter");
    await expect
      .poll(
        async () =>
          page.evaluate(
            () => document.activeElement?.getAttribute("data-role") ?? null,
          ),
        { timeout: 5_000 },
      )
      .toBe("book-filter");
    await expect(bookFilter).toHaveValue("");

    expect(consoleErrors.filter((m) => !m.includes("favicon"))).toEqual([]);
  });
});
