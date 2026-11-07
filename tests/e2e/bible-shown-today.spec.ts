/**
 * #850 B2d, spec §7.4: "Táto bohoslužba", the verses shown today under the
 * console's field. Shared by every device, a tap only previews, and a chip
 * appearing after a show never moves Zobraziť on a desktop. Its own server:
 * the list starts empty and every test depends on what the earlier ones showed.
 */
import { test, expect, type Browser, type Page, type Request } from "@playwright/test";
import {
  assertMinTapTarget,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000, mode: "serial" });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.beforeEach(async ({ request }) => {
  const prefs = await request.put(new URL("/bible/preferences", baseURL).toString(), {
    data: { mainTranslation: "slk-seb", characterLimit: 320 },
  });
  expect(prefs.ok()).toBeTruthy();
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

async function open(browser: Browser, profile: object, errors: string[]): Promise<Page> {
  const context = await browser.newContext({ ...profile, baseURL });
  const page = await context.newPage();
  attachConsoleErrorCollector(page, errors);
  await page.goto("/ui/operator/bible");
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-books-translation",
    "slk-seb",
  );
  return page;
}

async function show(page: Page, text: string, reference: string): Promise<void> {
  await page.locator('[data-role="console-field"]').fill(text);
  const root = page.locator('[data-role="bible-console"]');
  await expect(root).toHaveAttribute("data-preview-reference", reference);
  await expect(root).toHaveAttribute("data-preview-ready", "true");
  await page.locator('[data-role="console-show"]').click();
  await expect(page.locator('[data-role="console-field"]')).toHaveValue("");
}

const chips = (page: Page) => page.locator('[data-role="console-shown-chip"]');

test("desktop: the list starts hidden, fills newest first, and a chip never moves Zobraziť", async ({
  browser,
}) => {
  const errors: string[] = [];
  const page = await open(browser, { viewport: { width: 1280, height: 720 } }, errors);
  await expect(page.locator('[data-role="console-shown"]')).toHaveCount(0);

  await page.locator('[data-role="console-field"]').fill("zid 11 1");
  await expect(page.locator('[data-role="bible-console"]')).toHaveAttribute(
    "data-preview-ready",
    "true",
  );
  const before = await page.locator('[data-role="console-show"]').boundingBox();
  await page.locator('[data-role="console-show"]').click();
  await expect(chips(page)).toHaveText(["Hebrejom 11:1"]);
  await expect(page.locator('[data-role="console-shown"]')).toContainText("Táto bohoslužba");
  const after = await page.locator('[data-role="console-show"]').boundingBox();
  expect(after?.y).toBe(before?.y);

  await show(page, "rim 8 28", "Rimanom 8:28 (SEB)");
  await expect(chips(page)).toHaveText(["Rimanom 8:28", "Hebrejom 11:1"]);

  // A repeat moves up, it is not a second entry.
  await show(page, "zid 11 1", "Hebrejom 11:1 (SEB)");
  await expect(chips(page)).toHaveText(["Hebrejom 11:1", "Rimanom 8:28"]);

  // Text in the field hides the list, an empty field brings it back.
  await page.locator('[data-role="console-field"]').fill("zal");
  await expect(page.locator('[data-role="console-shown"]')).toHaveCount(0);
  await page.locator('[data-role="console-field"]').fill("");
  await expect(chips(page)).toHaveCount(2);
  expect(errors).toEqual([]);
  await page.context().close();
});

test("phone: a verse another device showed arrives; a tap previews and only Zobraziť sends", async ({
  browser,
  request,
}) => {
  const errors: string[] = [];
  const phone = await open(browser, PHONE_390, errors);
  await expect(chips(phone)).toHaveCount(2);
  await assertMinTapTarget(chips(phone).first(), 44);

  // Another device's show reaches this phone without a reload, by the wall's event.
  const trigger = await request.post(new URL("/bible/trigger", baseURL).toString(), {
    data: { translation: "slk-seb", book: "Hebrejom", bookCode: "HEB", bookNumber: 58, chapter: 12, verseStart: 1 },
  });
  expect(trigger.ok()).toBeTruthy();
  await expect(chips(phone).first()).toHaveText("Hebrejom 12:1");

  const posts: Request[] = [];
  phone.on("request", (req) => {
    if (req.method() === "POST" && new URL(req.url()).pathname.startsWith("/bible/trigger")) {
      posts.push(req);
    }
  });
  const root = phone.locator('[data-role="bible-console"]');
  const rimanom = chips(phone).filter({ hasText: "Rimanom 8:28" });
  await rimanom.tap();
  await expect(root).toHaveAttribute("data-preview-ready", "true");
  await expect(root).toHaveAttribute("data-preview-reference", "Rimanom 8:28 (SEB)");
  await expect(rimanom).toHaveAttribute("aria-pressed", "true");
  expect(posts).toHaveLength(0);

  await phone.locator('[data-role="console-show"]').tap();
  await expect(chips(phone).first()).toHaveText("Rimanom 8:28");
  expect(posts).toHaveLength(1);
  expect(errors).toEqual([]);
  await phone.context().close();
});

test("a chip follows a translation switch: the same verse in the new translation's words", async ({
  browser,
}) => {
  const errors: string[] = [];
  const page = await open(browser, { viewport: { width: 1280, height: 720 } }, errors);
  await page.locator('.console-translation[data-kind="main"][data-translation="eng-kjv"]').click();
  const root = page.locator('[data-role="bible-console"]');
  await expect(root).toHaveAttribute("data-books-translation", "eng-kjv");
  await chips(page).filter({ hasText: "Hebrejom 11:1" }).click();
  await expect(root).toHaveAttribute("data-preview-ready", "true");
  await expect(root).toHaveAttribute("data-preview-reference", "Hebrews 11:1 (KJV)");
  expect(errors).toEqual([]);
  await page.context().close();
});
