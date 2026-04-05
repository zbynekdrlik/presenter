/**
 * Tests that follow mode scrolls the slide list to keep the active slide visible.
 */
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
let dbUrl: string;

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(config.port, dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
});

test("follow mode scrolls active slide into view", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Find a presentation with enough slides to require scrolling
  const libsResp = await page.request.get(
    new URL("/libraries", baseURL).toString(),
  );
  expect(libsResp.status()).toBe(200);
  const libraries = await libsResp.json();
  expect(libraries.length).toBeGreaterThan(0);

  let targetPresId: string | null = null;
  let targetSlides: any[] = [];
  for (const lib of libraries) {
    const presResp = await page.request.get(
      new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    );
    const presentations = await presResp.json();
    for (const pres of presentations) {
      const detailResp = await page.request.get(
        new URL(`/presentations/${pres.id}`, baseURL).toString(),
      );
      const detail = await detailResp.json();
      if (detail.presentation.slides.length >= 8) {
        targetPresId = pres.id;
        targetSlides = detail.presentation.slides;
        break;
      }
    }
    if (targetPresId) break;
  }

  test.skip(!targetPresId, "No presentation with 8+ slides found");

  // Enable follow mode
  await page.request.post(
    new URL("/integrations/ableset/follow", baseURL).toString(),
    { data: { enabled: true } },
  );

  // Navigate to operator
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Trigger the LAST slide in the presentation (should be off-screen)
  const lastSlide = targetSlides[targetSlides.length - 1];
  await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: targetPresId,
        currentSlideId: lastSlide.id,
        nextSlideId: null,
      },
    },
  );

  // Wait for the slide list to load and scroll effect to fire
  await page.waitForTimeout(2000);

  // Verify the active slide card is visible and has is-active class
  const activeCard = page.locator(
    `.operator__slides [data-slide-id="${lastSlide.id}"]`,
  );
  await expect(activeCard).toBeVisible({ timeout: 5000 });
  await expect(activeCard).toHaveClass(/is-active/);

  // Verify the card is actually within the visible scroll area
  const isInView = await activeCard.evaluate((el) => {
    const container = el.closest(".operator__slides");
    if (!container) return false;
    const containerRect = container.getBoundingClientRect();
    const elRect = el.getBoundingClientRect();
    return (
      elRect.top >= containerRect.top - 10 &&
      elRect.bottom <= containerRect.bottom + 10
    );
  });
  expect(isInView).toBe(true);

  expect(
    consoleMessages.filter((m) => !m.includes("favicon")),
  ).toEqual([]);
});
