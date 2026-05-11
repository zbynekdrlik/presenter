import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(
  page: import("@playwright/test").Page,
  extraAllowed: RegExp[] = [],
): string[] {
  const messages: string[] = [];
  const allowed = [...ALLOWED_CONSOLE_NOISE, ...extraAllowed];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!allowed.some((pattern) => pattern.test(text))) {
        messages.push(`[${msg.type()}] ${text}`);
      }
    }
  });
  return messages;
}

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

/**
 * Helper: open the operator in edit mode, select the first available
 * presentation, and wait for slides to render.
 * Returns the slide_id of the first slide.
 *
 * NOTE: Worship slide cards use `data-slide-id` on the article element;
 * there is no `data-role="slide-card"`. Edit mode is a global mode toggle
 * (`[data-role="mode-toggle"][data-mode="edit"]`), not a per-slide button.
 */
async function openOperatorWithEditingSlide(
  page: import("@playwright/test").Page,
): Promise<string> {
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // First library, first presentation
  await page.waitForSelector('[data-role="library-item"]', { timeout: 30_000 });
  await page.locator('[data-role="library-item"]').first().click();

  await page.waitForSelector('[data-role="presentation-item"]', {
    timeout: 15_000,
  });
  await page.locator('[data-role="presentation-item"]').first().click();

  // Wait for slides to appear
  await page.waitForFunction(
    () => {
      const slides = document.querySelectorAll("[data-slide-id]");
      return slides.length > 0;
    },
    { timeout: 15_000 },
  );

  // Allow async presentation detail fetch to complete
  await page
    .waitForResponse(
      (resp) => resp.url().includes("/presentations/") && resp.status() === 200,
      { timeout: 10_000 },
    )
    .catch(() => {}); // May have already completed

  // Switch to edit mode
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
    { timeout: 5_000 },
  );

  // Wait for textarea to materialise in the first slide card
  await page.waitForSelector('[data-slide-id] textarea[data-field="main"]', {
    timeout: 5_000,
  });

  // Return the slide-id of the first slide
  const slideId =
    (await page.locator("[data-slide-id]").first().getAttribute("data-slide-id")) ?? "";
  return slideId;
}

test("Saved indicator appears and fades after blur", async ({ page }) => {
  const consoleMessages = collectConsoleErrors(page);

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-slide-id="${slideId}"]`);

  // Type into the main textarea
  const main = slideCard.locator('textarea[data-field="main"]');
  await main.click();
  await main.press("End");
  await main.type(" autosave probe", { delay: 20 });

  // Blur by clicking outside any editable region
  await page.locator("body").click({ position: { x: 5, y: 5 } });

  // Indicator must appear within 3s
  const indicator = slideCard.locator('[data-role="slide-save-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 3_000 });
  await expect(indicator).toHaveAttribute("data-status", "saved");
  await expect(indicator).toHaveText(/Saved/);

  // Indicator must fade (disappear) within 5s after that
  await expect(indicator).toHaveCount(0, { timeout: 5_000 });

  expect(consoleMessages).toEqual([]);
});

test("Save button is absent from slide controls (regression #313)", async ({
  page,
}) => {
  const consoleMessages = collectConsoleErrors(page);

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-slide-id="${slideId}"]`);

  // The Save button used to live in operator__slide-controls. After #313 it must be gone.
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="save"]'),
  ).toHaveCount(0);

  // Duplicate and Delete must remain.
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="duplicate"]'),
  ).toBeVisible();
  await expect(
    slideCard.locator('.operator__slide-controls button[data-action="delete"]'),
  ).toBeVisible();

  expect(consoleMessages).toEqual([]);
});

test("Save failed sticks when server returns 500", async ({ page }) => {
  // 500 on the slide-update PUT will produce a console error; allow it for this test.
  const consoleMessages = collectConsoleErrors(page, [
    /Failed to load resource.*500/i,
  ]);

  await page.route("**/presentations/*/slides/*", (route) => {
    if (route.request().method() === "PUT") {
      route.fulfill({ status: 500, body: "boom" });
    } else {
      route.continue();
    }
  });

  const slideId = await openOperatorWithEditingSlide(page);
  const slideCard = page.locator(`[data-slide-id="${slideId}"]`);

  const main = slideCard.locator('textarea[data-field="main"]');
  await main.click();
  await main.press("End");
  await main.type(" will fail", { delay: 20 });
  await page.locator("body").click({ position: { x: 5, y: 5 } });

  const indicator = slideCard.locator('[data-role="slide-save-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 3_000 });
  await expect(indicator).toHaveAttribute("data-status", "failed");
  await expect(indicator).toHaveText(/failed/i);

  // Must NOT fade within 5s
  await page.waitForTimeout(5_000);
  await expect(indicator).toBeVisible();
  await expect(indicator).toHaveAttribute("data-status", "failed");

  expect(consoleMessages).toEqual([]);
});
