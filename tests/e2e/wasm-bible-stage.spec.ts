/**
 * WASM Bible Stage Display Tests
 *
 * Verifies that triggering Bible verses from the WASM operator
 * actually displays text on the /stage page when using the dedicated
 * "bible" layout, and that clearing removes it. Bible text only shows
 * on the "bible" layout — not on worship/timer/preach layouts.
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

/** Set the stage layout via API. */
async function setLayout(
  request: import("@playwright/test").APIRequestContext,
  code: string,
) {
  await request.post(new URL("/stage/layout", baseURL).toString(), {
    data: { code },
  });
}

/** Navigate to stage, wait for WASM and WS connection. */
async function openStage(page: import("@playwright/test").Page) {
  await page.goto(`${baseURL}/stage`);
  await page.waitForLoadState("domcontentloaded");
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForFunction(
    () => window.__presenterStageConnectionState === "connected",
    { timeout: 30_000 },
  );
}

test("triggering bible slide shows text on bible layout", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);
  await setLayout(request, "bible");

  await triggerSlide(request, {
    mainText: "For God so loved the world",
    mainReference: "John 3:16 (NIV)",
  });

  await openStage(page);

  // Verify layout is bible
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    "bible",
  );

  // Bible content should be visible
  const bibleText = page.locator(".stage__bible-text");
  await expect(bibleText).toHaveText("For God so loved the world", {
    timeout: 10_000,
  });

  const bibleRef = page.locator(".stage__bible-reference");
  await expect(bibleRef).toHaveText("John 3:16 (NIV)");

  // Reset layout
  await setLayout(request, "worship-snv");
});

test("clearing bible broadcast hides text on bible layout", async ({
  page,
  request,
}) => {
  await setLayout(request, "bible");

  await triggerSlide(request, {
    mainText: "The Lord is my shepherd",
    mainReference: "Psalm 23:1 (NIV)",
  });

  await openStage(page);

  // Verify it's visible
  const bibleText = page.locator(".stage__bible-text");
  await expect(bibleText).toHaveText("The Lord is my shepherd", {
    timeout: 10_000,
  });

  // Clear the broadcast
  await clearBroadcast(request);

  // Waiting text should appear
  const waiting = page.locator(".stage__bible-waiting");
  await expect(waiting).toBeVisible({ timeout: 10_000 });

  // body should reflect no active bible
  await expect(page.locator("body")).toHaveAttribute(
    "data-bible-active",
    "false",
  );

  await setLayout(request, "worship-snv");
});

test("bible text does NOT show on worship-snv layout", async ({
  page,
  request,
}) => {
  await setLayout(request, "worship-snv");

  await triggerSlide(request, {
    mainText: "In the beginning was the Word",
    mainReference: "John 1:1 (ESV)",
  });

  await openStage(page);

  // Verify layout is worship-snv
  await expect(page.locator("body")).toHaveAttribute(
    "data-layout-code",
    "worship-snv",
  );

  // Bible overlay should NOT exist on this layout
  const bibleOverlay = page.locator(".stage__bible-overlay");
  await expect(bibleOverlay).toHaveCount(0);

  // Bible content (from bible layout) should NOT exist either
  const bibleContent = page.locator(".stage__bible-content");
  await expect(bibleContent).toHaveCount(0);
});

test("bible text does NOT show on preach layout", async ({ page, request }) => {
  await setLayout(request, "preach");

  await triggerSlide(request, {
    mainText: "I can do all things through Christ",
    mainReference: "Philippians 4:13 (NKJV)",
  });

  await openStage(page);

  // Bible overlay should NOT exist
  const bibleOverlay = page.locator(".stage__bible-overlay");
  await expect(bibleOverlay).toHaveCount(0);

  const bibleContent = page.locator(".stage__bible-content");
  await expect(bibleContent).toHaveCount(0);

  await setLayout(request, "worship-snv");
});

test("trigger with secondary translation shows both texts on bible layout", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);
  await setLayout(request, "bible");

  await triggerSlide(request, {
    mainText: "For God so loved the world",
    mainReference: "John 3:16 (NIV)",
    secondaryText: "Lebo tak Boh miloval svet",
    secondaryReference: "Ján 3:16 (ROH)",
  });

  await openStage(page);

  // Verify main text
  await expect(page.locator(".stage__bible-text")).toHaveText(
    "For God so loved the world",
    { timeout: 10_000 },
  );
  await expect(page.locator(".stage__bible-reference")).toHaveText(
    "John 3:16 (NIV)",
  );

  // Verify secondary text is visible
  const secondary = page.locator(".stage__bible-secondary");
  await expect(secondary).toHaveAttribute("data-visible", "true");

  await expect(page.locator(".stage__bible-secondary-text")).toHaveText(
    "Lebo tak Boh miloval svet",
  );
  await expect(page.locator(".stage__bible-secondary-ref")).toHaveText(
    "Ján 3:16 (ROH)",
  );

  await setLayout(request, "worship-snv");
});

test("secondary translation hidden when not provided", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);
  await setLayout(request, "bible");

  await triggerSlide(request, {
    mainText: "Be still and know that I am God",
    mainReference: "Psalm 46:10 (NIV)",
  });

  await openStage(page);

  await expect(page.locator(".stage__bible-text")).toHaveText(
    "Be still and know that I am God",
    { timeout: 10_000 },
  );

  // Secondary should be hidden
  const secondary = page.locator(".stage__bible-secondary");
  await expect(secondary).toHaveAttribute("data-visible", "false");

  await setLayout(request, "worship-snv");
});

test("rapid trigger-clear-trigger cycle works correctly", async ({
  page,
  request,
}) => {
  await clearBroadcast(request);
  await setLayout(request, "bible");

  await openStage(page);

  // First trigger
  await triggerSlide(request, {
    mainText: "First verse text",
    mainReference: "Gen 1:1",
  });

  await expect(page.locator(".stage__bible-text")).toHaveText(
    "First verse text",
    { timeout: 10_000 },
  );

  // Clear
  await clearBroadcast(request);
  await expect(page.locator(".stage__bible-waiting")).toBeVisible({
    timeout: 10_000,
  });

  // Second trigger with different text
  await triggerSlide(request, {
    mainText: "Second verse text",
    mainReference: "Gen 1:2",
  });

  await expect(page.locator(".stage__bible-text")).toHaveText(
    "Second verse text",
    { timeout: 10_000 },
  );
  await expect(page.locator(".stage__bible-reference")).toHaveText("Gen 1:2");

  await setLayout(request, "worship-snv");
});

test("stage layout change to bible shows active broadcast", async ({
  page,
  request,
}) => {
  // Start on worship-snv
  await setLayout(request, "worship-snv");

  // Trigger a verse while on worship layout
  await triggerSlide(request, {
    mainText: "The truth shall set you free",
    mainReference: "John 8:32 (NIV)",
  });

  // Switch to bible layout
  await setLayout(request, "bible");

  await openStage(page);

  // Bible text should be visible (fetched on page load)
  await expect(page.locator(".stage__bible-text")).toHaveText(
    "The truth shall set you free",
    { timeout: 10_000 },
  );

  await setLayout(request, "worship-snv");
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

// Type declarations for window object
declare global {
  interface Window {
    __presenterStageConnectionState?: string;
  }
}
