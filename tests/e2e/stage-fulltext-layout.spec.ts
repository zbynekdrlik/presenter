import { test, expect, BrowserContext, Page } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

/**
 * #515 — fulltext stage layout + per-slide stage-layout markers.
 *
 * (a) The `fulltext` layout renders ONLY the current slide's stage text,
 *     auto-scaled to the whole screen (short text ⇒ larger font than long).
 * (b) A slide carrying a stage-layout marker switches the stage layout when
 *     triggered (server-side, like POST /stage/layout); unmarked slides
 *     leave the layout untouched.
 * (c) The operator can assign a marker from the slide card (edit mode) and
 *     sees a badge on marked slides (live mode).
 * All tests assert a clean browser console.
 */

test.describe.configure({ timeout: 180_000 });

const ALLOWED_CONSOLE_NOISE = [
  /integrity.*ignored.*preload/i,
  /ResizeObserver loop/i,
];

function collectConsoleErrors(page: Page): string[] {
  const messages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      if (!ALLOWED_CONSOLE_NOISE.some((pattern) => pattern.test(text))) {
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

const LIBRARY_NAME = "_E2E Fulltext";
const SHORT_STAGE_TEXT = "Read this short message";
const LONG_STAGE_TEXT = [
  "This is a deliberately long hand-off text for the speaker to read from",
  "the stage display. It spans multiple sentences so the autofit binary",
  "search has to shrink the font significantly below the size it picks for",
  "a short message, which is exactly what this end-to-end test asserts.",
  "The text keeps going for a while to make the contrast unmistakable and",
  "the assertion robust against small rendering differences between runs.",
].join(" ");
const FALLBACK_MAIN_TEXT = "Fallback main text";

let presentationId = "";
let slideIds: string[] = [];

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);

  const libResp = await fetch(new URL("/libraries", baseURL).toString(), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name: LIBRARY_NAME }),
  });
  const lib = await libResp.json();

  const presResp = await fetch(
    new URL(`/libraries/${lib.id}/presentations`, baseURL).toString(),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: "Fulltext Cases",
        slides: [
          { main: "Slide one main", stage: SHORT_STAGE_TEXT },
          { main: "Slide two main", stage: LONG_STAGE_TEXT },
          { main: FALLBACK_MAIN_TEXT },
        ],
      }),
    },
  );
  const presData = await presResp.json();
  presentationId = presData.presentation.id;
  slideIds = presData.presentation.slides.map((s: { id: string }) => s.id);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

async function setLayout(context: BrowserContext, code: string) {
  const resp = await context.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code } },
  );
  expect(resp.status()).toBe(200);
}

async function triggerSlide(context: BrowserContext, idx: number) {
  const resp = await context.request.post(
    new URL("/stage/state", baseURL).toString(),
    {
      data: {
        presentationId,
        currentSlideId: slideIds[idx],
        nextSlideId: slideIds[idx + 1] ?? null,
      },
    },
  );
  expect(resp.status()).toBe(204);
}

async function openStage(context: BrowserContext): Promise<Page> {
  const page = await context.newPage();
  await page.goto(new URL("/stage", baseURL).toString(), {
    waitUntil: "domcontentloaded",
  });
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  return page;
}

async function fulltextFontPx(page: Page): Promise<number> {
  return page
    .locator('[data-role="fulltext-text"]')
    .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
}

test("fulltext layout shows the stage text fullscreen and auto-scales it", async ({
  context,
}) => {
  await setLayout(context, "fulltext");
  const stagePage = await openStage(context);
  const consoleErrors = collectConsoleErrors(stagePage);

  await stagePage.waitForSelector('body[data-layout-code="fulltext"]', {
    timeout: 15_000,
  });

  // Short stage text renders verbatim in the fulltext area.
  await triggerSlide(context, 0);
  const text = stagePage.locator('[data-role="fulltext-text"]');
  await expect(text).toHaveText(SHORT_STAGE_TEXT, { timeout: 15_000 });
  await expect(text).toBeVisible();

  // Autofit picked a real font size (non-trivially large for a short text).
  await expect
    .poll(() => fulltextFontPx(stagePage), { timeout: 15_000 })
    .toBeGreaterThan(60);
  const shortFont = await fulltextFontPx(stagePage);

  // The long hand-off text must scale DOWN substantially to fit the screen.
  await triggerSlide(context, 1);
  await expect(text).toHaveText(LONG_STAGE_TEXT, { timeout: 15_000 });
  await expect
    .poll(() => fulltextFontPx(stagePage), { timeout: 15_000 })
    .toBeLessThan(shortFont * 0.6);
  const longFont = await fulltextFontPx(stagePage);
  expect(longFont).toBeGreaterThan(1);

  // A slide with an empty stage field falls back to its main text.
  await triggerSlide(context, 2);
  await expect(text).toHaveText(FALLBACK_MAIN_TEXT, { timeout: 15_000 });

  expect(consoleErrors).toEqual([]);
  await stagePage.close();
});

test("triggering a marked slide switches the stage layout; unmarked slides keep it", async ({
  context,
}) => {
  await setLayout(context, "worship-snv");
  const stagePage = await openStage(context);
  const consoleErrors = collectConsoleErrors(stagePage);

  await stagePage.waitForSelector('body[data-layout-code="worship-snv"]', {
    timeout: 15_000,
  });

  // Assign the fulltext marker to slide 0 via the API.
  const putResp = await context.request.put(
    new URL(
      `/presentations/${presentationId}/slides/${slideIds[0]}/stage-layout`,
      baseURL,
    ).toString(),
    { data: { layoutCode: "fulltext" } },
  );
  expect(putResp.status()).toBe(204);

  // Triggering the marked slide flips the live stage to fulltext.
  await triggerSlide(context, 0);
  await stagePage.waitForSelector('body[data-layout-code="fulltext"]', {
    timeout: 15_000,
  });
  await expect(
    stagePage.locator('[data-role="fulltext-text"]'),
  ).toHaveText(SHORT_STAGE_TEXT, { timeout: 15_000 });

  // Triggering an UNMARKED slide keeps the layout (content still updates).
  await triggerSlide(context, 2);
  await expect(
    stagePage.locator('[data-role="fulltext-text"]'),
  ).toHaveText(FALLBACK_MAIN_TEXT, { timeout: 15_000 });
  await expect(stagePage.locator("body")).toHaveAttribute(
    "data-layout-code",
    "fulltext",
  );

  // An unknown layout code is rejected outright.
  const badResp = await context.request.put(
    new URL(
      `/presentations/${presentationId}/slides/${slideIds[0]}/stage-layout`,
      baseURL,
    ).toString(),
    { data: { layoutCode: "no-such-layout" } },
  );
  expect(badResp.status()).toBe(400);

  expect(consoleErrors).toEqual([]);
  await stagePage.close();
});

test("operator assigns a marker from the slide card and sees the badge in live mode", async ({
  context,
}) => {
  const page = await context.newPage();
  const consoleErrors = collectConsoleErrors(page);

  await page.goto(new URL("/ui/operator", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // Open the test library (created after seeding, so it is not in the
  // favorites sidebar — go through the "Show all libraries" modal) and the
  // test presentation.
  await page.waitForSelector('[data-role="library-more"]', {
    timeout: 30_000,
  });
  await page.locator('[data-role="library-more"]').click();
  await page
    .locator('[data-role="library-row"]', { hasText: LIBRARY_NAME })
    .locator("button.operator__list-button")
    .click();
  await page
    .locator('[data-role="presentation-item"]', { hasText: "Fulltext Cases" })
    .first()
    .click();
  await page.waitForSelector(`[data-slide-id="${slideIds[1]}"]`, {
    timeout: 30_000,
  });

  // Edit mode exposes the per-slide stage-layout selector.
  await page.locator('[data-role="mode-toggle"][data-mode="edit"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "edit",
  );
  const select = page
    .locator(`[data-slide-id="${slideIds[1]}"]`)
    .locator('[data-role="slide-stage-layout-select"]');
  await expect(select).toBeVisible({ timeout: 15_000 });
  await select.selectOption("fulltext");

  // The marker lands on the server.
  await expect
    .poll(async () => {
      const resp = await context.request.get(
        new URL(
          `/presentations/${presentationId}/slide-stage-layouts`,
          baseURL,
        ).toString(),
      );
      const map = (await resp.json()) as Record<string, string>;
      return map[slideIds[1]];
    })
    .toBe("fulltext");

  // Live mode shows the badge on the marked slide.
  await page.locator('[data-role="mode-toggle"][data-mode="live"]').click();
  await page.waitForFunction(
    () => document.body.getAttribute("data-mode") === "live",
  );
  const badge = page
    .locator(`[data-slide-id="${slideIds[1]}"]`)
    .locator('[data-role="slide-stage-layout-badge"]');
  await expect(badge).toBeVisible({ timeout: 15_000 });
  await expect(badge).toContainText("FULL TEXT");

  expect(consoleErrors).toEqual([]);
  await page.close();
});
