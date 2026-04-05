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
let testPresId: string;
let testSlideIds: string[] = [];

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(config.port, dbUrl, config.oscPort);

  // Create a test library and presentation with 10 slides
  const libResp = await fetch(
    new URL("/libraries", baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "_E2E Follow Scroll" }),
    },
  );
  const lib = await libResp.json();

  const slides = Array.from({ length: 10 }, (_, i) => ({
    main: `Slide ${i + 1}\nLine two of slide ${i + 1}\nLine three`,
  }));

  const presResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Scroll Test Song", slides }),
    },
  );
  const presData = await presResp.json();
  testPresId = presData.presentation.id;
  testSlideIds = presData.presentation.slides.map(
    (s: { id: string }) => s.id,
  );
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

  // Enable follow mode
  await page.request.post(
    new URL("/integrations/ableset/follow", baseURL).toString(),
    { data: { enabled: true } },
  );

  // Navigate to operator and wait for WASM + WS connection
  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  // Wait for WebSocket connection to establish
  await page.waitForTimeout(1000);

  // Trigger the LAST slide (index 9) — should be off-screen
  const lastSlideId = testSlideIds[testSlideIds.length - 1];
  await page.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId: testPresId,
        currentSlideId: lastSlideId,
        nextSlideId: null,
      },
    },
  );

  // Wait for follow to auto-navigate, slides to load, and scroll effect
  await page.waitForTimeout(4000);

  // Verify the active slide card is visible and has is-active class
  const activeCard = page.locator(
    `.operator__slides [data-slide-id="${lastSlideId}"]`,
  );
  await expect(activeCard).toBeVisible({ timeout: 5000 });
  await expect(activeCard).toHaveClass(/is-active/);

  // Verify the card is within the visible scroll area of its container
  // Allow generous tolerance for smooth scroll animation completion
  const isInView = await activeCard.evaluate((el) => {
    const container = el.closest(".operator__slides");
    if (!container) return false;
    const containerRect = container.getBoundingClientRect();
    const elRect = el.getBoundingClientRect();
    // At least partially visible (top of card is above bottom of container)
    return elRect.top < containerRect.bottom && elRect.bottom > containerRect.top;
  });
  expect(isInView).toBe(true);

  expect(
    consoleMessages.filter((m) => !m.includes("favicon")),
  ).toEqual([]);
});
