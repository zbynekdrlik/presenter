/**
 * WASM Bible Stage Display Tests
 *
 * Verifies that triggering Bible verses from the WASM operator
 * actually displays text on the /stage page, and that clearing
 * removes it. These tests check the REAL stage rendering,
 * not just API responses or toast messages.
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

/** Trigger a Bible slide via the API and return the response. */
async function triggerSlide(
  request: import("@playwright/test").APIRequestContext,
  options: {
    mainText: string;
    mainReference: string;
    secondaryText?: string;
    secondaryReference?: string;
  },
) {
  const resp = await request.post(
    new URL("/bible/trigger-slide", baseURL).toString(),
    {
      data: {
        mainText: options.mainText,
        mainReference: options.mainReference,
        secondaryText: options.secondaryText ?? "",
        secondaryReference: options.secondaryReference ?? "",
      },
    },
  );
  expect(resp.ok()).toBeTruthy();
  return resp.json();
}

/** Clear the Bible broadcast via the API. */
async function clearBroadcast(
  request: import("@playwright/test").APIRequestContext,
) {
  const resp = await request.post(new URL("/bible/clear", baseURL).toString());
  expect(resp.status()).toBe(204);
}

test("triggering bible slide shows text on stage page", async ({
  page,
  request,
}) => {
  // Clear any existing broadcast
  await clearBroadcast(request);

  // Trigger a Bible slide
  await triggerSlide(request, {
    mainText: "For God so loved the world",
    mainReference: "John 3:16 (NIV)",
  });

  // Navigate to stage page
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Wait for the Bible overlay to become visible with the triggered text
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });

  // Verify the actual Bible text is displayed
  const bibleText = page.locator("#bible-text");
  await expect(bibleText).toHaveText("For God so loved the world");

  // Verify the reference is displayed
  const bibleRef = page.locator("#bible-reference");
  await expect(bibleRef).toHaveText("John 3:16 (NIV)");

  // Verify the overlay is visually covering the stage (positional check)
  const overlayBox = await bibleOverlay.boundingBox();
  expect(overlayBox).toBeTruthy();
  if (overlayBox) {
    expect(overlayBox.width).toBeGreaterThan(100);
    expect(overlayBox.height).toBeGreaterThan(100);
  }
});

test("clearing bible broadcast hides text on stage page", async ({
  page,
  request,
}) => {
  // Trigger a slide first
  await triggerSlide(request, {
    mainText: "The Lord is my shepherd",
    mainReference: "Psalm 23:1 (NIV)",
  });

  // Navigate to stage
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Verify it's visible
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });

  // Clear the broadcast
  await clearBroadcast(request);

  // Wait for the overlay to disappear
  await expect(bibleOverlay).toHaveAttribute("data-visible", "false", {
    timeout: 10_000,
  });

  // body should reflect no active bible
  await expect(page.locator("body")).toHaveAttribute(
    "data-bible-active",
    "false",
  );
});

test("bible overlay works on worship-snv layout", async ({ page, request }) => {
  // Set layout to worship-snv (default)
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });

  // Trigger a Bible slide
  await triggerSlide(request, {
    mainText: "In the beginning was the Word",
    mainReference: "John 1:1 (ESV)",
  });

  // Navigate to stage
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Verify layout is worship-snv
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    "worship-snv",
  );

  // Verify Bible overlay is visible with correct text
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(page.locator("#bible-text")).toHaveText(
    "In the beginning was the Word",
  );
});

test("bible overlay works on preach layout", async ({ page, request }) => {
  // Set layout to preach
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "preach" },
  });

  // Trigger a Bible slide
  await triggerSlide(request, {
    mainText: "I can do all things through Christ",
    mainReference: "Philippians 4:13 (NKJV)",
  });

  // Navigate to stage with preach layout
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Verify Bible overlay is visible
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(page.locator("#bible-text")).toHaveText(
    "I can do all things through Christ",
  );
  await expect(page.locator("#bible-reference")).toHaveText(
    "Philippians 4:13 (NKJV)",
  );

  // Reset layout to default
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
});

test("trigger with secondary translation shows both texts on stage", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);

  // Trigger with secondary translation
  await triggerSlide(request, {
    mainText: "For God so loved the world",
    mainReference: "John 3:16 (NIV)",
    secondaryText: "Lebo tak Boh miloval svet",
    secondaryReference: "Ján 3:16 (ROH)",
  });

  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Wait for overlay
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });

  // Verify main text
  await expect(page.locator("#bible-text")).toHaveText(
    "For God so loved the world",
  );
  await expect(page.locator("#bible-reference")).toHaveText("John 3:16 (NIV)");

  // Verify secondary text is visible
  const secondary = page.locator("#bible-secondary");
  await expect(secondary).toHaveAttribute("data-visible", "true");

  await expect(page.locator("#bible-secondary-text")).toHaveText(
    "Lebo tak Boh miloval svet",
  );
  await expect(page.locator("#bible-secondary-ref")).toHaveText(
    "Ján 3:16 (ROH)",
  );
});

test("secondary translation hidden when not provided", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);

  // Trigger WITHOUT secondary translation
  await triggerSlide(request, {
    mainText: "Be still and know that I am God",
    mainReference: "Psalm 46:10 (NIV)",
  });

  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });

  // Secondary should be hidden
  const secondary = page.locator("#bible-secondary");
  await expect(secondary).toHaveAttribute("data-visible", "false");
});

test("rapid trigger-clear-trigger cycle works correctly", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);

  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // First trigger
  await triggerSlide(request, {
    mainText: "First verse text",
    mainReference: "Gen 1:1",
  });

  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(page.locator("#bible-text")).toHaveText("First verse text");

  // Clear
  await clearBroadcast(request);
  await expect(bibleOverlay).toHaveAttribute("data-visible", "false", {
    timeout: 10_000,
  });

  // Second trigger with different text
  await triggerSlide(request, {
    mainText: "Second verse text",
    mainReference: "Gen 1:2",
  });

  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(page.locator("#bible-text")).toHaveText("Second verse text");
  await expect(page.locator("#bible-reference")).toHaveText("Gen 1:2");
});

test("stage layout change preserves active bible broadcast", async ({
  page,
  request,
}) => {
  // Set initial layout
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });

  // Trigger a verse
  await triggerSlide(request, {
    mainText: "The truth shall set you free",
    mainReference: "John 8:32 (NIV)",
  });

  // Navigate to stage
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");

  // Verify verse is visible
  const bibleOverlay = page.locator("#bible-overlay");
  await expect(bibleOverlay).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });

  // Change layout — this causes a page reload
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "preach" },
  });

  // Wait for page to reload (layout change triggers reload)
  await page.waitForTimeout(2000);
  await page.waitForLoadState("domcontentloaded");

  // After reload, bible overlay should still show the active verse
  // (because fetchBibleActive runs on page load)
  const overlayAfter = page.locator("#bible-overlay");
  await expect(overlayAfter).toHaveAttribute("data-visible", "true", {
    timeout: 10_000,
  });
  await expect(page.locator("#bible-text")).toHaveText(
    "The truth shall set you free",
  );

  // Clean up: reset layout
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code: "worship-snv" },
  });
});

test("active-slide API endpoint returns current slide output", async ({
  request,
}) => {
  await clearBroadcast(request);

  // No active slide initially
  const emptyResp = await request.get(
    new URL("/bible/active-slide", baseURL).toString(),
  );
  expect(emptyResp.ok()).toBeTruthy();
  const emptyBody = await emptyResp.json();
  expect(emptyBody).toBeNull();

  // Trigger a slide
  await triggerSlide(request, {
    mainText: "Test slide text",
    mainReference: "Test 1:1",
    secondaryText: "Secondary test",
    secondaryReference: "Test 1:1 (B)",
  });

  // Active slide should return the output
  const activeResp = await request.get(
    new URL("/bible/active-slide", baseURL).toString(),
  );
  expect(activeResp.ok()).toBeTruthy();
  const activeBody = await activeResp.json();
  expect(activeBody.mainText).toBe("Test slide text");
  expect(activeBody.mainReference).toBe("Test 1:1");
  expect(activeBody.secondaryText).toBe("Secondary test");
  expect(activeBody.secondaryReference).toBe("Test 1:1 (B)");

  // Clear and verify
  await clearBroadcast(request);
  const clearedResp = await request.get(
    new URL("/bible/active-slide", baseURL).toString(),
  );
  expect(clearedResp.ok()).toBeTruthy();
  const clearedBody = await clearedResp.json();
  expect(clearedBody).toBeNull();
});
