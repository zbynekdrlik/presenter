/**
 * #938 — the home page `/` on a phone. The owner opened prsnv.newlevel.media on
 * his phone and had to zoom to reach the operator and its AI tab: the page had
 * no viewport meta and small inline chips. It is a phone page now: big
 * full-width links, the most used first, a direct link to the AI tab.
 */
import { test, expect } from "@playwright/test";
import {
  assertNoHorizontalOverflow,
  attachConsoleErrorCollector,
  deriveTestConfig,
  PHONE_390,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  serverHandle = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("the home page is a phone page: big links, most used first, the AI tab one tap away", async ({
  browser,
}) => {
  const context = await browser.newContext({ ...PHONE_390, baseURL });
  const page = await context.newPage();
  const errors: string[] = [];
  attachConsoleErrorCollector(page, errors);
  await page.goto("/");

  // Laid out at the phone's width, not a zoomed-out desktop page.
  expect(await page.evaluate(() => window.innerWidth)).toBe(390);
  await assertNoHorizontalOverflow(page);

  const links = page.locator("a[href]");
  const boxes = await links.evaluateAll((anchors) =>
    anchors.map((anchor) => {
      const rect = anchor.getBoundingClientRect();
      return { href: anchor.getAttribute("href"), width: rect.width, height: rect.height };
    }),
  );
  expect(boxes.length).toBeGreaterThanOrEqual(12);
  for (const box of boxes) {
    expect(box.height, `${box.href} is too low to tap`).toBeGreaterThanOrEqual(48);
    expect(box.width, `${box.href} is not a full-width row`).toBeGreaterThanOrEqual(340);
  }
  // The most used surfaces first (SNV's /ui/usage, 90 days).
  expect(boxes.slice(0, 4).map((box) => box.href)).toEqual([
    "/ui/operator",
    "/ui/study",
    "/ui/operator/ai",
    "/ui/tablet",
  ]);
  await expect(page.locator('a[href="/ui/operator"]')).toContainText("Operátor");
  await expect(page.locator('a[href="/ui/study"]')).toContainText("Štúdium");
  await expect(page.locator("html")).toHaveAttribute("lang", "sk");

  // One tap opens the operator straight on its AI tab.
  await page.locator('a[href="/ui/operator/ai"]').tap();
  await page.waitForSelector('body[data-wasm-ready="true"]', { state: "attached", timeout: 30_000 });
  await expect(page.locator("body")).toHaveAttribute("data-view", "ai");
  await expect(page.locator('[data-role="ai-input"]')).toBeVisible();

  expect(errors).toEqual([]);
  await context.close();
});
