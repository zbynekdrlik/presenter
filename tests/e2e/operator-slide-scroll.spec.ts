/**
 * E2E tests for issue #271 — operator slide-list scroll UX.
 *
 * Three concerns:
 * 1. Lookahead: clicking a slide ensures the next row is visible below.
 * 2. Linear wheel: each wheel notch scrolls a deterministic step regardless
 *    of deltaY magnitude (neutralises macOS acceleration).
 * 3. Load-at-start: opening a new presentation scrolls the slide list to top.
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
let dbUrl: string;

/** Presentation with 15 slides for scroll tests. */
let presId15: string;
/** Second presentation with 12 slides for load-at-start test. */
let presId12: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  dbUrl = config.dbUrl;
  await refreshDevData(dbUrl);
  serverHandle = await startTestServer(config.port, dbUrl, config.oscPort);

  // Create a test library with two presentations so all tests share seed data.
  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: "_E2E Slide Scroll" }),
  });
  const lib = await libResp.json();

  // 15-slide presentation (enough for 5 rows in a 3-column grid)
  const slides15 = Array.from({ length: 15 }, (_, i) => ({
    main: `Verse ${i + 1}\nLine two of verse ${i + 1}\nLine three`,
  }));
  const pres15Resp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Scroll Test Song A", slides: slides15 }),
    },
  );
  const pres15Data = await pres15Resp.json();
  presId15 = pres15Data.presentation.id;

  // 12-slide presentation for load-at-start test
  const slides12 = Array.from({ length: 12 }, (_, i) => ({
    main: `Chorus ${i + 1}\nLine two of chorus ${i + 1}\nLine three`,
  }));
  const pres12Resp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Scroll Test Song B", slides: slides12 }),
    },
  );
  const pres12Data = await pres12Resp.json();
  presId12 = pres12Data.presentation.id;
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

/**
 * Navigate to the operator page and open the given presentation by seeding
 * sessionStorage before WASM boots. Waits for slide cards to appear.
 *
 * Note: the Rust session module (gloo-storage 0.3) prefixes all keys with
 * "presenter:" and serialises values as JSON strings, so the raw sessionStorage
 * value is the JSON-encoded string (e.g. `"\"uuid\""` not `"uuid"`).
 */
async function openPresentation(
  page: import("@playwright/test").Page,
  presId: string,
): Promise<void> {
  await page.goto(new URL("/ui/operator", baseURL).toString());
  // Seed sessionStorage with gloo-storage-compatible format so WASM reads the
  // presentation id on init: prefix = "presenter:", value = JSON.stringify(id).
  await page.evaluate((id) => {
    sessionStorage.setItem("presenter:currentPresentationId", JSON.stringify(id));
    // Ensure worship view is active (default, but set explicitly for clarity).
    sessionStorage.setItem("presenter:view", JSON.stringify("worship"));
  }, presId);
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  // Wait for slide cards rendered by the WASM after it fetches the presentation.
  await page.waitForSelector(".operator__slides .operator__slide-card", {
    state: "visible",
    timeout: 30_000,
  });
}

test("lookahead: clicking a slide makes next-row slide visible", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await openPresentation(page, presId15);

  const cards = await page.locator(".operator__slides [data-slide-id]").all();
  expect(cards.length).toBeGreaterThanOrEqual(12);

  // Click slide at index 3 — first slide of row 2 in a 3-column grid.
  await cards[3].click();
  // Allow the scroll Effect (which runs after click) to settle.
  await page.waitForTimeout(300);

  // The next-row anchor for index 3 is index 6 (index + COLUMNS_PER_ROW=3).
  // It must be visible within the container (bottom <= container.bottom).
  const visibility = await page.evaluate(() => {
    const container = document.querySelector(".operator__slides");
    const allCards = document.querySelectorAll(
      ".operator__slides [data-slide-id]",
    );
    if (!container || allCards.length < 7) return null;
    const cRect = container.getBoundingClientRect();
    const anchorRect = (allCards[6] as Element).getBoundingClientRect();
    return {
      visible: anchorRect.bottom <= cRect.bottom + 2,
      anchorBottom: anchorRect.bottom,
      containerBottom: cRect.bottom,
    };
  });

  expect(visibility).not.toBeNull();
  expect(visibility!.visible).toBeTruthy();

  expect(
    consoleMessages.filter(
      (m) => !m.includes("favicon") && !m.includes("crbug.com/981419"),
    ),
  ).toEqual([]);
});

test("wheel: each notch scrolls a deterministic step", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await openPresentation(page, presId15);

  // Reset scroll to top so we start from a known position.
  await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (c) c.scrollTop = 0;
  });
  await page.waitForTimeout(50);

  // Dispatch a WheelEvent with deltaY=100. The wheel handler should ignore the
  // deltaY magnitude and instead apply step = card_height + 14.4 (grid gap).
  // If the handler is passive (Leptos on:wheel default), prevent_default is
  // silently ignored and the browser may apply deltaY=100 instead of the step.
  const result = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (!c) return null;
    const cardEl = c.querySelector(
      ".operator__slide-card",
    ) as HTMLElement | null;
    // Compute the expected step from the actual card height.
    const cardHeight = cardEl ? cardEl.getBoundingClientRect().height : 0;
    const expectedStep = cardHeight > 0 ? cardHeight + 14.4 : 120;

    const before = c.scrollTop;
    c.dispatchEvent(
      new WheelEvent("wheel", { deltaY: 100, bubbles: true, cancelable: true }),
    );
    const after = c.scrollTop;
    return { before, after, expectedStep, deltaY: 100 };
  });

  expect(result).not.toBeNull();

  const actualDelta = result!.after - result!.before;

  // If actualDelta equals deltaY (100), the handler was passive and
  // prevent_default had no effect — the browser applied raw native scroll.
  // The spec requires escalating this to BLOCKED.
  expect(
    actualDelta,
    `BLOCKED: wheel handler appears passive — scrollTop delta=${actualDelta} equals deltaY (100) instead of step (${result!.expectedStep.toFixed(1)}). Fix: change on:wheel in slide_list.rs to addEventListener with passive:false.`,
  ).not.toBe(result!.deltaY);

  // The handler must apply the deterministic step (±2px tolerance for rounding).
  expect(actualDelta).toBeGreaterThan(result!.expectedStep - 2);
  expect(actualDelta).toBeLessThan(result!.expectedStep + 2);

  expect(
    consoleMessages.filter(
      (m) => !m.includes("favicon") && !m.includes("crbug.com/981419"),
    ),
  ).toEqual([]);
});

test("load-at-start: opening a new presentation scrolls slide list to top", async ({
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Open the 15-slide presentation and scroll it to the bottom.
  await openPresentation(page, presId15);
  await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    if (c) c.scrollTop = c.scrollHeight;
  });
  await page.waitForTimeout(50);

  const scrolledDown = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    return c?.scrollTop ?? 0;
  });
  expect(scrolledDown).toBeGreaterThan(0);

  // Now switch to the 12-slide presentation. The WASM's Effect on
  // selected_presentation (issue #271 concern 3) should scroll the list to top.
  await page.evaluate((id) => {
    sessionStorage.setItem("presenter:currentPresentationId", JSON.stringify(id));
  }, presId12);
  await page.reload();
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector(".operator__slides .operator__slide-card", {
    state: "visible",
    timeout: 30_000,
  });

  // Give the scroll-to-top Effect (scheduled via Timeout(0)) time to run.
  await page.waitForTimeout(300);

  const scrollAfterSwitch = await page.evaluate(() => {
    const c = document.querySelector(".operator__slides") as HTMLElement | null;
    return c?.scrollTop ?? -1;
  });
  expect(scrollAfterSwitch).toBe(0);

  expect(
    consoleMessages.filter(
      (m) => !m.includes("favicon") && !m.includes("crbug.com/981419"),
    ),
  ).toEqual([]);
});
