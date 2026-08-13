import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

// #693: When the operator live-edits a slide's text, the tablet receives a
// `BibleSlidesChanged` WS event and re-renders the slide list. Before the fix,
// the list was rebuilt from scratch (unkeyed `collect_view()`), which tore down
// the whole `.tablet-main` scroll subtree and reset the user's scroll position
// (and moved the clicked/active slide). This spec proves the scroll offset and
// the marked slide's on-screen position survive a live content edit.

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("tablet keeps scroll position and marked slide in place across an operator live edit", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // --- Setup: create a Bible presentation with MANY slides (must overflow) ---
  const presentationName = `LiveEditScroll E2E ${Date.now()}`;
  const createResponse = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    {
      data: { name: presentationName },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(createResponse.ok()).toBeTruthy();
  const presentationId: string = (await createResponse.json()).id;

  // Resolve a broad verse range so the list is comfortably taller than the
  // (deliberately short) viewport below.
  const resolveResponse = await request.post(
    new URL("/bible/resolve", baseURL).toString(),
    {
      data: {
        mainTranslation: "eng-kjv",
        book: "John",
        bookCode: "JHN",
        chapter: 3,
        verseStart: 1,
        verseEnd: 36,
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(resolveResponse.ok()).toBeTruthy();
  const resolvedSlides: Array<{
    bibleMain: string;
    bibleTranslation: string;
    metadata?: unknown;
    bibleMainReference: string;
    bibleTranslationReference: string;
  }> = (await resolveResponse.json()).slides;
  expect(resolvedSlides.length).toBeGreaterThan(0);

  const appendResponse = await request.post(
    new URL(`/bible/presentations/${presentationId}/append`, baseURL).toString(),
    {
      data: {
        slides: resolvedSlides.map((slide) => ({
          bibleMain: slide.bibleMain,
          bibleTranslation: slide.bibleTranslation,
          bibleMainReference: slide.bibleMainReference,
          bibleTranslationReference: slide.bibleTranslationReference,
          metadata: slide.metadata || null,
        })),
      },
      headers: { "Content-Type": "application/json" },
      timeout: 60_000,
    },
  );
  expect(appendResponse.ok()).toBeTruthy();

  const detail = await (
    await request.get(
      new URL(`/bible/presentations/${presentationId}`, baseURL).toString(),
      { timeout: 60_000 },
    )
  ).json();
  const slides: Array<{
    id: string;
    bibleMain: string;
    bibleTranslation: string;
    bibleMainReference: string;
    bibleTranslationReference: string;
  }> = detail.slides;
  const slideCount = slides.length;
  // Need enough slides to guarantee the container scrolls.
  expect(slideCount).toBeGreaterThanOrEqual(4);

  // Target = a middle slide (the one the user "clicked/marked" and watches).
  const targetIdx = Math.floor(slideCount / 2);
  const targetId = slides[targetIdx].id;
  // Edit target = the LAST slide, which sits BELOW the target — its height
  // change cannot reflow anything above it, isolating scroll-preservation from
  // layout reflow.
  const editSlide = slides[slideCount - 1];
  const editMarker = "LIVE-EDIT-693";

  // --- Navigate to tablet UI, short viewport to force scrolling ---
  await page.setViewportSize({ width: 900, height: 420 });
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });
  await page.waitForSelector('[data-role="presentation-list"]', {
    state: "visible",
    timeout: 20_000,
  });

  // Select our presentation and wait for its slides to render.
  const presentationButton = page.locator(
    `[data-role="presentation-button"][data-presentation-id="${presentationId}"]`,
  );
  await presentationButton.waitFor({ state: "visible", timeout: 10_000 });
  await presentationButton.click();
  const slideCards = page.locator('[data-role="tablet-slide"]');
  await expect(slideCards).toHaveCount(slideCount, { timeout: 15_000 });

  const targetCard = page.locator(
    `[data-role="tablet-slide"][data-slide-id="${targetId}"]`,
  );
  const scrollContainer = page.locator(".tablet-main");

  // Mark the target: click it to trigger, and wait until it's active.
  await targetCard.click();
  await expect(targetCard).toHaveClass(/is-active/, { timeout: 10_000 });

  // Scroll so the marked slide sits mid-viewport, then record baseline.
  await targetCard.evaluate((el) =>
    el.scrollIntoView({
      block: "center",
      behavior: "instant" as ScrollBehavior,
    }),
  );
  await page.waitForTimeout(200); // let scroll settle
  const scrollBefore = await scrollContainer.evaluate((el) => el.scrollTop);
  // The scroll must be real, or the test wouldn't exercise the bug at all.
  expect(scrollBefore).toBeGreaterThan(50);
  const boxBefore = await targetCard.boundingBox();
  expect(boxBefore).not.toBeNull();
  const yBefore = boxBefore!.y;

  // --- Operator live-edits the LAST slide's text via the HTTP API ---
  const editResponse = await request.patch(
    new URL(
      `/bible/presentations/${presentationId}/slides/${editSlide.id}`,
      baseURL,
    ).toString(),
    {
      data: {
        bibleMain: `${editSlide.bibleMain} ${editMarker}`,
        bibleTranslation: editSlide.bibleTranslation,
        bibleMainReference: editSlide.bibleMainReference,
        bibleTranslationReference: editSlide.bibleTranslationReference,
      },
      headers: { "Content-Type": "application/json" },
      timeout: 30_000,
    },
  );
  expect(editResponse.ok()).toBeTruthy();

  // Wait until the edit has actually re-rendered on the tablet (proves the
  // WS-update → re-render path ran; retries cover WS connect latency).
  await expect(
    page.locator(
      `[data-role="tablet-slide"][data-slide-id="${editSlide.id}"] .tablet-slide__main`,
    ),
  ).toContainText(editMarker, { timeout: 20_000 });

  // --- Assert: scroll offset and marked-slide position are unchanged ---
  const scrollAfter = await scrollContainer.evaluate((el) => el.scrollTop);
  expect(Math.abs(scrollAfter - scrollBefore)).toBeLessThanOrEqual(2);

  const boxAfter = await targetCard.boundingBox();
  expect(boxAfter).not.toBeNull();
  expect(Math.abs(boxAfter!.y - yBefore)).toBeLessThanOrEqual(2);

  // The marked slide must still be marked after the live edit.
  await expect(targetCard).toHaveClass(/is-active/);

  expect(consoleMessages).toEqual([]);
});
